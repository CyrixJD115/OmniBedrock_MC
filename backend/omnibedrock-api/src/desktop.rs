//! Tauri desktop shell (`--desktop`, built with `--features desktop`).
//!
//! The HTTP server still serves the UI and API from this same process; the
//! native window is simply a WebKit view of it, so there is one binary and no
//! separate frontend protocol to keep in sync.
use crate::server_proc;
use anyhow::Context as _;
use omnibedrock_core::config::{Cli, Config};
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tokio_util::sync::CancellationToken;

/// Load config, start the panel server, then block on the Tauri event loop
/// until the window closes; the server shuts down cleanly afterwards.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    // WebKitGTK's DMABUF renderer causes severe scroll/input lag and rendering
    // artifacts on many Linux GPU stacks (NVIDIA and hybrid graphics above
    // all); the plain renderer is the standard workaround. Must be set before
    // WebKitGTK initializes, and a value the user exported wins.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // Single-threaded at this point (before the runtime and window exist).
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    let config = Config::load(cli)?;
    let _log_guard = server_proc::init_tracing(&config);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build async runtime")?;
    let shutdown = CancellationToken::new();
    let (state, address, server) =
        runtime.block_on(server_proc::spawn(config, shutdown.clone()))?;
    // A 0.0.0.0 bind serves every interface; the window only needs loopback.
    let host = if state.config.host.is_unspecified() {
        std::net::IpAddr::from([127, 0, 0, 1])
    } else {
        state.config.host
    };
    let url: tauri::Url = format!("http://{host}:{}", address.port())
        .parse()
        .context("parse panel URL")?;
    tracing::info!(%url, "opening desktop window");
    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("OmniBedrock MC")
                .inner_size(1280.0, 800.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .context("Tauri desktop app failed")?;
    tracing::info!("Desktop window closed; shutting down the panel server");
    shutdown.cancel();
    runtime.block_on(server)?;
    Ok(())
}
