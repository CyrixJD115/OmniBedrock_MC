//! The Endstone manager handle: a control-channel worker that serializes
//! toolchain installs panel-wide, plus install-state tracking and the cached
//! release registry reads.
use crate::server::Console;
use anyhow::Context;
use omnibedrock_core::config::Config;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use super::install::run_install;
use super::pypi::{EndstoneRegistry, REGISTRY_TTL, fetch_registry_cached};

enum Control {
    Install {
        version: Option<String>,
        reply: tokio::sync::oneshot::Sender<anyhow::Result<String>>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct Endstone {
    control: mpsc::Sender<Control>,
    pub state: watch::Receiver<crate::bds::InstallState>,
    http: reqwest::Client,
    registry: Arc<tokio::sync::RwLock<Option<(std::time::Instant, EndstoneRegistry)>>>,
    config: Arc<Config>,
}

impl Endstone {
    pub fn new(
        config: Arc<Config>,
        console: Console,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Self {
        let (state_tx, state) = watch::channel(crate::bds::InstallState::idle());
        let (control, control_rx) = mpsc::channel(4);
        let http = reqwest::Client::builder()
            .user_agent(concat!("OmniBedrock/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(600))
            .build()
            .expect("reqwest client");
        let manager = Self {
            control,
            state,
            http: http.clone(),
            registry: Arc::new(tokio::sync::RwLock::new(None)),
            config,
        };
        tokio::spawn(worker(
            manager.config.clone(),
            control_rx,
            state_tx,
            http,
            manager.registry.clone(),
            console,
            shutdown,
        ));
        manager
    }
    pub fn installed_marker(config: &Config) -> PathBuf {
        config.data_dir.join("endstone_toolchain.json")
    }
    pub async fn installed(&self) -> Option<Value> {
        let text = tokio::fs::read_to_string(Self::installed_marker(&self.config))
            .await
            .ok()?;
        serde_json::from_str(&text).ok()
    }
    /// The Endstone version recorded in the install marker, if any.
    pub async fn installed_version(&self) -> Option<String> {
        let installed = self.installed().await?;
        installed["endstone_version"].as_str().map(str::to_string)
    }
    /// Fetch the Endstone release registry (cached for `REGISTRY_TTL`).
    pub async fn registry(&self) -> anyhow::Result<EndstoneRegistry> {
        fetch_registry_cached(&self.http, &self.registry).await
    }
    /// The cached registry without touching the network; `None` until the
    /// first successful fetch. Keeps hot paths (status polling) offline.
    pub fn cached_registry(&self) -> Option<EndstoneRegistry> {
        self.registry
            .try_read()
            .ok()?
            .as_ref()
            .filter(|(at, _)| at.elapsed() < REGISTRY_TTL)
            .map(|(_, r)| r.clone())
    }
    /// Queue a toolchain installation; the reply resolves when it finishes.
    pub async fn install(
        &self,
        version: Option<String>,
    ) -> anyhow::Result<tokio::sync::oneshot::Receiver<anyhow::Result<String>>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.control
            .send(Control::Install { version, reply: tx })
            .await
            .context("installer stopped")?;
        Ok(rx)
    }
    pub async fn shutdown(&self) {
        let _ = self.control.send(Control::Shutdown).await;
    }
}

async fn worker(
    config: Arc<Config>,
    mut control: mpsc::Receiver<Control>,
    state: watch::Sender<crate::bds::InstallState>,
    http: reqwest::Client,
    registry: Arc<tokio::sync::RwLock<Option<(std::time::Instant, EndstoneRegistry)>>>,
    console: Console,
    shutdown: tokio_util::sync::CancellationToken,
) {
    loop {
        let message = tokio::select! {
            _ = shutdown.cancelled() => break,
            m = control.recv() => match m {
                Some(m) => m,
                None => break,
            },
        };
        match message {
            Control::Shutdown => break,
            Control::Install { version, reply } => {
                if state.borrow().phase != "idle" {
                    let _ = reply.send(Err(anyhow::anyhow!(
                        "another Endstone install is already running"
                    )));
                    continue;
                }
                // Every instance shares one toolchain under <data_dir>/tools;
                // serialize installs panel-wide, then re-check in case another
                // instance's install just finished the same work.
                // run_install resolves "latest" through the shared registry
                // cache and no-ops when the toolchain is already current, so
                // update requests and start-bootstraps are safe to queue here.
                static TOOLCHAIN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
                let _guard = TOOLCHAIN.lock().await;
                let result =
                    run_install(&config, &state, &http, &registry, &console, version).await;
                state.send_modify(|s| {
                    s.phase = "idle".into();
                    s.progress = None;
                });
                let _ = reply.send(result);
            }
        }
    }
}
