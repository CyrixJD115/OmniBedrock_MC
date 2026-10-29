//! Process bootstrap shared by both modes: logging, service startup, and the
//! HTTP listener that serves the embedded UI and the API from one process.
use crate::api::{AppState, router};
use anyhow::Context as _;
use omnibedrock_core::config::{Cli, Config};
use std::net::SocketAddr;
use tokio_util::sync::CancellationToken;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Configure tracing the same way for every launch mode. Keep the returned
/// guard alive for the whole process or file logs are silently dropped.
pub fn init_tracing(config: &Config) -> WorkerGuard {
    let file = tracing_appender::rolling::never(&config.logs_dir, "omnibedrock.log");
    let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .lossy(false)
        .finish(file);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                if config.debug {
                    "omnibedrock=debug,tower_http=debug".into()
                } else {
                    "omnibedrock=info,tower_http=info".into()
                }
            }),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        )
        .init();
    guard
}

/// Bring up every service and run the HTTP server in the background. Returns
/// the bound address so callers (desktop mode) can point a window at it; the
/// handle resolves once the server has fully stopped after a shutdown.
pub async fn spawn(
    config: Config,
    shutdown: CancellationToken,
) -> anyhow::Result<(AppState, SocketAddr, tokio::task::JoinHandle<()>)> {
    let boot = std::time::Instant::now();
    config.ensure_dirs().await?;
    tracing::info!(
        portable = config.portable,
        data_mode = %config.data_mode,
        config_source = %config.config_source,
        host = %config.host,
        port = config.port,
        bedrock_dir = %config.bedrock_server_dir.display(),
        data_dir = %config.data_dir.display(),
        logs_dir = %config.logs_dir.display(),
        backups_dir = %config.backups_dir.display(),
        cache_dir = %config.cache_dir.display(),
        broadcaster_dir = %config.broadcaster_dir.display(),
        "configuration loaded"
    );
    let state = crate::api::AppState::new(config, shutdown.clone()).await?;
    // Until first-run setup completes (the seeded default password is still
    // active), a wildcard bind would expose Admin/<default> to the whole
    // network; pin the listener to loopback and tell the operator how to get
    // the configured address back.
    let mut host = state.config.host;
    if state.auth.default_credentials_active() && host.is_unspecified() {
        host = std::net::IpAddr::from(std::net::Ipv4Addr::LOCALHOST);
        tracing::warn!(
            configured_host = %state.config.host,
            "Default admin credentials are still active — binding the panel to              {host} instead of the configured address. Change the default              password (or set ADMIN_PASSWORD before first boot) to listen on it."
        );
    }
    let address = SocketAddr::new(host, state.config.port);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("bind API listener on {address}"))?;
    let app = router(state.clone())?;
    // Fingerprint of the embedded dashboard (build.rs hashes frontend/build
    // into the binary); matches GET /api/v1/build-id byte for byte.
    tracing::info!("UI build: {}", crate::bundle::UI_BUILD_HASH);
    tracing::info!(
        %address,
        startup_ms = boot.elapsed().as_millis() as u64,
        "OmniBedrock panel ready — UI and API served from one process"
    );
    let stopping = state.clone();
    let handle = tokio::spawn(async move {
        let boot = boot;
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal(shutdown))
        .await
        {
            tracing::error!(%e, "HTTP server failed");
        }
        tracing::info!("Shutdown requested");
        stopping.shutdown.cancel();
        // Hooks must complete/resume saves before shutting down instances.
        for instance in stopping.instances.list().await {
            instance.backups.wait_idle().await;
        }
        let _lock = stopping.operations.lock().await;
        stopping.instances.shutdown_all().await;
        tracing::info!(
            uptime_ms = boot.elapsed().as_millis() as u64,
            "shutdown complete"
        );
    });
    Ok((state, address, handle))
}

/// Default mode: a pure web panel. Blocks until Ctrl+C / SIGTERM.
#[tokio::main]
pub async fn run_standalone(cli: Cli) -> anyhow::Result<()> {
    let config = Config::load(cli)?;
    let _guard = init_tracing(&config);
    let shutdown = CancellationToken::new();
    let (_state, address, server) = spawn(config, shutdown).await?;
    tracing::info!(
        "Open the panel at http://localhost:{} (or use --desktop for a window)",
        address.port()
    );
    server.await?;
    Ok(())
}

async fn shutdown_signal(token: CancellationToken) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = token.cancelled() => {},
    }
}
