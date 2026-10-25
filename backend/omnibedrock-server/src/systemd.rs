//! systemd user-service runtime for instances.
//!
//! A `systemd` instance runs inside a persistent user unit
//! `omnibedrock-<id>.service` (never a system unit: the panel does not need
//! root, and user units keep server processes owned by the panel's own
//! login). The unit's `ExecStart` points back at the panel binary running
//! the hidden `__unit-helper` subcommand, which wires the server's stdin to
//! a FIFO the panel (and only the panel) writes commands into, forwards
//! stdout/stderr to the journal, and performs graceful `stop`-then-signal
//! shutdown when systemd asks the service to stop.
//!
//! Because the unit persists on disk, the server survives panel restarts
//! and can start at boot (`autostart` → `systemctl --user enable` plus
//! loginctl linger). The panel re-attaches console output (`journalctl -f`)
//! and exit monitoring when it comes back.
//!
//! Unit files always carry a managed marker; an existing file without it is
//! never overwritten.

use crate::instances::ResourcePolicy;
use anyhow::{Context, bail};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

/// Header line present in every unit the panel writes. A file without it is
/// foreign and must never be replaced silently.
pub const MANAGED_MARKER: &str = "# Managed by OmniBedrock";

/// `systemctl --user` invocations must never hang the panel.
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(15);

pub fn unit_name(id: &str) -> String {
    format!("omnibedrock-{id}.service")
}

/// Directory holding user units: `$XDG_CONFIG_HOME/systemd/user` or
/// `~/.config/systemd/user`.
pub fn unit_dir() -> anyhow::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .context("$HOME is not set; cannot locate the systemd user directory")?;
            home.join(".config")
        }
    };
    Ok(base.join("systemd").join("user"))
}

pub fn unit_path(id: &str) -> anyhow::Result<PathBuf> {
    Ok(unit_dir()?.join(unit_name(id)))
}

/// Everything needed to render one instance unit.
pub struct UnitSpec<'a> {
    pub instance_id: &'a str,
    pub description: &'a str,
    /// Panel executable; re-resolved at every launch so moved binaries heal.
    pub exe: &'a Path,
    /// FIFO the helper reads server commands from.
    pub fifo: &'a Path,
    pub working_dir: &'a Path,
    /// Server argv (backend binary + its arguments), argv[0] included.
    pub argv: &'a [OsString],
    pub resources: ResourcePolicy,
    /// Graceful stop budget the helper uses before escalating to signals.
    pub stop_timeout: u64,
    pub kill_timeout: u64,
}

/// Quote one value for a systemd unit assignment / ExecStart argument:
/// double quotes, escaping the characters systemd's parser treats specially
/// (`"`, `\`, and `$` which would otherwise trigger variable expansion).
fn quote(value: &std::ffi::OsStr) -> String {
    // Escape backslashes first, then quotes (each would otherwise create a
    // new backslash), and dollars (systemd would expand them as variables).
    let escaped = value
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "$$");
    if escaped.contains(['"', '\\', '$', ' ', '\t']) {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

pub fn render_unit(spec: &UnitSpec) -> String {
    let mut out = String::new();
    out.push_str("# Managed by OmniBedrock; the panel regenerates this file.\n");
    out.push_str("[Unit]\n");
    out.push_str(&format!(
        "Description=OmniBedrock instance {} ({})\n",
        spec.instance_id,
        spec.description.replace('\n', " ")
    ));
    out.push_str("After=network.target\n\n");
    out.push_str("[Service]\n");
    out.push_str("Type=simple\n");
    out.push_str(&format!(
        "WorkingDirectory={}\n",
        quote(spec.working_dir.as_os_str())
    ));
    let mut exec = format!(
        "ExecStart={} __unit-helper --stdin-fifo {} --",
        quote(spec.exe.as_os_str()),
        quote(spec.fifo.as_os_str())
    );
    for arg in spec.argv {
        exec.push(' ');
        exec.push_str(&quote(arg));
    }
    out.push_str(&exec);
    out.push('\n');
    // Only the helper process receives systemd's stop signal; it forwards a
    // graceful `stop` to the server first and escalates on its own.
    out.push_str("KillMode=process\n");
    out.push_str(&format!(
        "TimeoutStopSec={}\n",
        spec.stop_timeout + spec.kill_timeout + 5
    ));
    out.push_str("Restart=no\n");
    if let Some(mb) = spec.resources.memory_mb {
        out.push_str(&format!("MemoryMax={mb}M\n"));
        out.push_str("MemorySwapMax=0\n");
    }
    if let Some(percent) = spec.resources.cpu_quota_percent() {
        out.push_str(&format!("CPUQuota={percent}%\n"));
    }
    out.push_str("\n[Install]\nWantedBy=default.target\n");
    out
}

/// Decide what to do with an existing unit file. `Ok(None)` = identical
/// content, `Ok(Some(new))` = managed file that should be replaced, `Err` =
/// foreign file the panel refuses to touch.
pub fn plan_install(existing: &str, content: &str) -> anyhow::Result<Option<String>> {
    if existing == content {
        return Ok(None);
    }
    if !existing.contains(MANAGED_MARKER) {
        bail!(
            "unit file exists but is not managed by OmniBedrock (missing '{MANAGED_MARKER}' header); \
             refusing to overwrite a foreign unit — remove or rename it first"
        );
    }
    Ok(Some(existing.to_string()))
}

/// Write the unit file if needed and reload the user manager. Returns true
/// when the file changed on disk.
pub async fn install_unit(id: &str, content: &str) -> anyhow::Result<bool> {
    let path = unit_path(id)?;
    if let Ok(existing) = std::fs::read_to_string(&path)
        && plan_install(&existing, content)?.is_none()
    {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, content)
        .await
        .with_context(|| format!("write unit {}", path.display()))?;
    daemon_reload().await?;
    Ok(true)
}

/// Remove a managed unit: disable it, delete the file, reload. Foreign
/// files are reported, not deleted.
pub async fn remove_unit(id: &str) -> anyhow::Result<()> {
    let path = unit_path(id)?;
    if !path.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(&path)?;
    if !existing.contains(MANAGED_MARKER) {
        bail!(
            "unit file {} is not managed by OmniBedrock; left in place",
            path.display()
        );
    }
    let _ = systemctl(&["disable", "--now", &unit_name(id)]).await;
    tokio::fs::remove_file(&path)
        .await
        .with_context(|| format!("remove unit {}", path.display()))?;
    daemon_reload().await
}

async fn systemctl(args: &[&str]) -> anyhow::Result<String> {
    let output = tokio::time::timeout(
        SYSTEMCTL_TIMEOUT,
        Command::new("systemctl")
            .arg("--user")
            .args(args)
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .context("systemctl --user timed out")?
    .context("failed to run systemctl")?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("systemctl --user {} failed: {err}", args.join(" "));
    }
    Ok(text)
}

pub async fn daemon_reload() -> anyhow::Result<()> {
    systemctl(&["daemon-reload"]).await.map(|_| ())
}

/// Best-effort reset so a previously failed unit can start again.
pub async fn reset_failed(id: &str) {
    let _ = systemctl(&["reset-failed", &unit_name(id)]).await;
}

pub async fn start_unit(id: &str) -> anyhow::Result<()> {
    systemctl(&["start", &unit_name(id)]).await.map(|_| ())
}

pub async fn stop_unit(id: &str) -> anyhow::Result<()> {
    systemctl(&["stop", &unit_name(id)]).await.map(|_| ())
}

pub async fn kill_unit(id: &str) -> anyhow::Result<()> {
    systemctl(&["kill", &unit_name(id), "--signal=SIGKILL"])
        .await
        .map(|_| ())
}

pub async fn set_enabled(id: &str, enabled: bool) -> anyhow::Result<()> {
    let action = if enabled { "enable" } else { "disable" };
    systemctl(&[action, &unit_name(id)]).await.map(|_| ())
}

/// Fetch unit properties as a map of `key=value` lines.
async fn show_properties(
    id: &str,
    props: &[&str],
) -> Option<std::collections::BTreeMap<String, String>> {
    let mut args = vec!["show".to_string(), unit_name(id)];
    for p in props {
        args.push(format!("--property={p}"));
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let text = systemctl(&args).await.ok()?;
    let mut map = std::collections::BTreeMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=')
            && !v.is_empty()
        {
            map.insert(k.to_string(), v.to_string());
        }
    }
    Some(map)
}

/// `ActiveState` if the unit exists (any state, including `failed`).
/// `systemctl show` also answers for unknown units (reporting `inactive`),
/// so callers that need a really-running unit must compare against
/// `"active"` themselves.
pub async fn active_state(id: &str) -> Option<String> {
    show_properties(id, &["ActiveState"])
        .await
        .and_then(|m| m.get("ActiveState").cloned())
}

/// MainPID of a running unit. PID 0 is systemd's placeholder for "no main
/// process" (nonexistent or stopped unit) and is reported as `None`.
pub async fn main_pid(id: &str) -> Option<u32> {
    show_properties(id, &["MainPID"])
        .await
        .and_then(|m| m.get("MainPID").and_then(|v| v.parse().ok()))
        .filter(|pid| *pid != 0)
}

/// Exit status of the last run (`ExecMainStatus`) once the unit left active.
pub async fn exit_status(id: &str) -> Option<i32> {
    show_properties(id, &["ExecMainStatus"])
        .await
        .and_then(|m| m.get("ExecMainStatus").and_then(|v| v.parse().ok()))
}

/// A user systemd manager exists and accepts commands (i.e. the panel runs
/// inside a user session with `$DBUS_SESSION_BUS_ADDRESS` or XDG runtime).
pub async fn user_manager_available() -> bool {
    tokio::time::timeout(
        Duration::from_secs(3),
        Command::new("systemctl")
            .args(["--user", "show-environment"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await
    .map(|s| s.is_ok_and(|status| status.success()))
    .unwrap_or(false)
}

/// Whether the user's units keep running with no login session (linger).
pub async fn linger_enabled() -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new("loginctl")
            .args(["show-user", &user, "--property=Linger", "--value"])
            .stdin(Stdio::null())
            .output(),
    )
    .await;
    matches!(
        output,
        Ok(Ok(out)) if out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "yes"
    )
}

/// Ensure the command FIFO exists next to the instance state.
pub async fn ensure_fifo(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match tokio::fs::symlink_metadata(path).await {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                nix::unistd::mkfifo(
                    path,
                    nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
                )
                .map_err(|e| anyhow::anyhow!("create FIFO {}: {e}", path.display()))?;
            }
            #[cfg(not(unix))]
            {
                anyhow::bail!("systemd instances require a Linux host");
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec<'a>(argv: &'a [OsString]) -> UnitSpec<'a> {
        UnitSpec {
            instance_id: "test-instance",
            description: "Dad's Server",
            exe: Path::new("/opt/omni bedrock/omnibedrock"),
            fifo: Path::new("/srv/data/instances/test-instance/stdin.fifo"),
            working_dir: Path::new("/srv/data/instances/test-instance/server"),
            argv,
            resources: ResourcePolicy {
                memory_mb: Some(1024),
                cpus: Some(2.0),
            },
            stop_timeout: 8,
            kill_timeout: 4,
        }
    }

    #[test]
    fn unit_names_are_flat() {
        assert_eq!(
            unit_name("dad-s-server"),
            "omnibedrock-dad-s-server.service"
        );
    }

    #[test]
    fn renders_quoted_paths_limits_and_marker() {
        let argv: Vec<OsString> =
            vec!["/srv/data/instances/test-instance/server/bedrock_server".into()];
        let text = render_unit(&spec(&argv));
        assert!(text.starts_with(MANAGED_MARKER));
        assert!(text.contains("Description=OmniBedrock instance test-instance (Dad's Server)"));
        assert!(text.contains("ExecStart=\"/opt/omni bedrock/omnibedrock\" __unit-helper"));
        // Plain paths stay unquoted; only specials (spaces, quotes, $) quote.
        assert!(text.contains("--stdin-fifo /srv/data/instances/test-instance/stdin.fifo"));
        assert!(text.contains("WorkingDirectory=/srv/data/instances/test-instance/server"));
        assert!(text.contains("MemoryMax=1024M"));
        assert!(text.contains("MemorySwapMax=0"));
        assert!(text.contains("CPUQuota=200%"));
        assert!(text.contains("KillMode=process"));
        assert!(text.contains("TimeoutStopSec=17"));
        assert!(text.contains("WantedBy=default.target"));
    }

    #[test]
    fn renders_without_limits_when_unlimited() {
        let argv: Vec<OsString> = vec!["/usr/bin/bedrock_server".into()];
        let mut s = spec(&argv);
        s.resources = ResourcePolicy::default();
        let text = render_unit(&s);
        assert!(!text.contains("MemoryMax"));
        assert!(!text.contains("CPUQuota"));
    }

    #[test]
    fn quoting_escapes_dollars_and_quotes() {
        assert_eq!(quote(std::ffi::OsStr::new("plain")), "plain");
        assert_eq!(quote(std::ffi::OsStr::new("with space")), "\"with space\"");
        assert_eq!(quote(std::ffi::OsStr::new("a$b\"c")), "\"a$$b\\\"c\"");
    }

    #[test]
    fn foreign_units_are_refused_identical_ones_are_noops() {
        let managed = format!("{MANAGED_MARKER}\n[Unit]\nDescription=x\n");
        assert!(plan_install(&managed, &managed).unwrap().is_none());
        let edited = format!("{managed}# tweak\n");
        assert!(plan_install(&edited, &managed).unwrap().is_some());
        let foreign = "# some admin unit\n[Unit]\nDescription=important\n";
        assert!(plan_install(foreign, &managed).is_err());
    }
}
