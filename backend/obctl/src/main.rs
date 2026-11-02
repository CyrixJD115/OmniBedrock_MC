//! obctl — one obvious command line for the OmniBedrock panel API: the same
//! operations agents perform over plain HTTP, wrapped for humans. Reads the
//! panel URL and bearer token from --url/--token or $OB_URL/$OB_TOKEN.
//!
//!     obctl login --username admin --password ...   # prints the token
//!     export OB_TOKEN=...
//!     obctl instances list
//!     obctl instances start default
//!     obctl command default list
//!     obctl broadcaster status
//!     obctl routes                                  # discovery aid
//!
//! Add --json to print raw response JSON instead of the human formatting.
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::time::Duration;

const API_PREFIX: &str = "/api/v1";

#[derive(Parser)]
#[command(
    name = "obctl",
    bin_name = "obctl",
    version,
    about = "Command-line client for the OmniBedrock panel API",
    after_help = "Environment: OB_URL (default http://127.0.0.1:17754), OB_TOKEN (bearer token).\nDiscover the full API at GET /api/v1 (route list) and /openapi.json."
)]
struct Cli {
    /// Panel base URL, without the /api/v1 prefix.
    #[arg(
        long,
        env = "OB_URL",
        value_name = "URL",
        default_value = "http://127.0.0.1:17754"
    )]
    url: String,
    /// Bearer token; falls back to $OB_TOKEN. `obctl login` mints one.
    #[arg(long, env = "OB_TOKEN", value_name = "TOKEN")]
    token: Option<String>,
    /// Print raw JSON responses instead of human formatting.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the panel's API routes (GET /api/v1).
    Routes,
    /// Exchange username + password for a bearer token.
    Login {
        #[arg(short = 'u', long)]
        username: String,
        #[arg(short = 'p', long)]
        password: String,
    },
    /// Instance operations.
    Instances {
        #[command(subcommand)]
        cmd: InstanceCmd,
    },
    /// Send one console command and print the server's response lines.
    Command {
        /// Instance id.
        #[arg(short, long, default_value = "default")]
        instance: String,
        /// The command (joined with spaces), e.g. `obctl command -i default list`.
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
    /// Broadcaster operations.
    Broadcaster {
        #[command(subcommand)]
        cmd: BroadcasterCmd,
    },
}

#[derive(Subcommand)]
enum InstanceCmd {
    /// List instances you can access.
    List,
    /// Show one instance's summary.
    Status { id: String },
    /// Start an instance (auto-installs the backend when missing).
    Start { id: String },
    /// Stop an instance gracefully.
    Stop { id: String },
    /// Restart an instance.
    Restart { id: String },
    /// Kill an instance without a graceful stop.
    Kill { id: String },
}

#[derive(Subcommand)]
enum BroadcasterCmd {
    /// Show the broadcaster integration status.
    Status,
}

#[tokio::main]
async fn main() {
    // A closed pipe (`obctl routes | head`) must exit quietly, not panic on
    // a failed stdout write — restore the default SIGPIPE disposition.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("obctl: {e:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let client = Client::new(&cli.url, cli.token.clone())?;
    match cli.cmd {
        Cmd::Routes => {
            let list = client.get("").await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            let Value::Array(routes) = list else {
                anyhow::bail!("route list response was not an array");
            };
            for route in routes {
                println!(
                    "{:6} {}  {}",
                    route["method"].as_str().unwrap_or_default(),
                    route["path"].as_str().unwrap_or_default(),
                    route["summary"].as_str().unwrap_or_default()
                );
            }
        }
        Cmd::Login { username, password } => {
            let body = client
                .post(
                    "/auth/login",
                    json!({"username": username, "password": password}),
                )
                .await?;
            let token = body["token"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("login response had no token"))?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&body)?);
            } else {
                println!("{token}");
                eprintln!("# usage: export OB_TOKEN={token}");
            }
        }
        Cmd::Instances { cmd } => match cmd {
            InstanceCmd::List => {
                let list = client.get("/instances").await?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&list)?);
                    return Ok(());
                }
                let Value::Array(instances) = list else {
                    anyhow::bail!("instances response was not an array");
                };
                let rows: Vec<[String; 6]> = instances
                    .iter()
                    .map(|i| {
                        [
                            i["id"].as_str().unwrap_or_default().to_owned(),
                            i["name"].as_str().unwrap_or_default().to_owned(),
                            i["backend"].as_str().unwrap_or_default().to_owned(),
                            i["status"].as_str().unwrap_or_default().to_owned(),
                            i["port"].as_u64().map_or(String::new(), |p| p.to_string()),
                            i["owner"].as_str().unwrap_or_default().to_owned(),
                        ]
                    })
                    .collect();
                print_table(&["ID", "NAME", "BACKEND", "STATUS", "PORT", "OWNER"], &rows);
            }
            InstanceCmd::Status { id } => {
                let body = client.get(&format!("/instances/{id}")).await?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&body)?);
                } else {
                    print_value(&body, 0);
                }
            }
            InstanceCmd::Start { id } => instance_action(&client, &id, "start", cli.json).await?,
            InstanceCmd::Stop { id } => instance_action(&client, &id, "stop", cli.json).await?,
            InstanceCmd::Restart { id } => {
                instance_action(&client, &id, "restart", cli.json).await?
            }
            InstanceCmd::Kill { id } => instance_action(&client, &id, "kill", cli.json).await?,
        },
        Cmd::Command { instance, command } => {
            let command = command.join(" ");
            if command.trim().is_empty() {
                anyhow::bail!("usage: obctl command -i <instance> <command...>");
            }
            let body = client
                .post(
                    &format!("/instances/{instance}/command"),
                    json!({"command": command}),
                )
                .await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&body)?);
                return Ok(());
            }
            match body["lines"].as_array() {
                Some(lines) => {
                    for line in lines {
                        println!("{}", line.as_str().unwrap_or_default());
                    }
                }
                None => print_value(&body, 0),
            }
        }
        Cmd::Broadcaster {
            cmd: BroadcasterCmd::Status,
        } => {
            let body = client.get("/broadcaster/status").await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&body)?);
            } else {
                print_value(&body, 0);
            }
        }
    }
    Ok(())
}

async fn instance_action(
    client: &Client,
    id: &str,
    action: &str,
    json_mode: bool,
) -> anyhow::Result<()> {
    let body = client
        .post(
            &format!("/instances/{id}/action"),
            json!({"action": action}),
        )
        .await?;
    if json_mode {
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    let message = body["message"].as_str().unwrap_or_default();
    if body["success"] == json!(false) {
        anyhow::bail!("{message}");
    }
    println!("{message}");
    Ok(())
}

struct Client {
    http: reqwest::Client,
    url: String,
    token: Option<String>,
}

impl Client {
    fn new(url: &str, token: Option<String>) -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()?,
            url: url.trim_end_matches('/').to_owned(),
            token,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{API_PREFIX}{path}", self.url)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        let response = request
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("panel unreachable at {}: {e}", self.url))?;
        let status = response.status();
        let text = response.text().await?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::String(text.clone()));
        if !status.is_success() {
            let message = value["error"]["message"]
                .as_str()
                .or(value["detail"].as_str())
                .unwrap_or("request failed");
            anyhow::bail!("HTTP {status}: {message}");
        }
        if value["ok"] == json!(false) {
            let message = value["error"]["message"].as_str().unwrap_or_default();
            anyhow::bail!("{message}");
        }
        Ok(value)
    }

    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        let mut request = self.http.get(self.endpoint(path));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        self.send(request).await
    }

    async fn post(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let mut request = self.http.post(self.endpoint(path)).json(&body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        self.send(request).await
    }
}

/// Aligned table with a dashed header separator, matching the panel's own
/// `?format=text` rendering.
fn print_table(headers: &[&str; 6], rows: &[[String; 6]]) {
    let widths: Vec<usize> = (0..6)
        .map(|col| {
            rows.iter()
                .map(|row| row[col].chars().count())
                .chain(std::iter::once(headers[col].len()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[String; 6]| {
        cells
            .iter()
            .enumerate()
            .map(|(col, cell)| format!("{cell:<width$}", width = widths[col]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    let header_cells: [String; 6] = headers.map(str::to_owned);
    let header_line = line(&header_cells);
    println!("{header_line}");
    println!("{}", "-".repeat(header_line.chars().count()));
    for row in rows {
        println!("{}", line(row));
    }
}

/// Human rendering of an arbitrary JSON body: scalars as `key: value`,
/// containers pretty-printed beneath their key, indented by depth.
fn print_value(value: &Value, depth: usize) {
    let indent = "  ".repeat(depth);
    let Some(object) = value.as_object() else {
        println!("{indent}{value}");
        return;
    };
    for (key, val) in object {
        match val {
            Value::String(s) => println!("{indent}{key}: {s}"),
            Value::Number(_) | Value::Bool(_) | Value::Null => println!("{indent}{key}: {val}"),
            Value::Array(items) if items.iter().all(is_scalar) => {
                let rendered = items
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("{indent}{key}: [{rendered}]");
            }
            other => {
                println!("{indent}{key}:");
                print_value(other, depth + 1);
            }
        }
    }
}

fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
    )
}
