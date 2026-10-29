//! Backend/server endpoints: backend status, BDS and Endstone installs,
//! start-time bootstrap, and backend switching.
use omnibedrock_core::{
    error::{ApiError, Result},
    files::component,
};
use omnibedrock_server::{
    bds, endstone,
    instances::{Deployment, Instance},
    server::{Backend, Status},
};
use serde_json::{Value, json};
use std::sync::Arc;

use super::helpers::{success, value_string};

pub(super) async fn backend_status(svc: &Arc<Instance>) -> Value {
    let settings = svc.server.settings.borrow().clone();
    let installed = svc.bds.installed_version().await;
    // Only surface a cached registry here; fetching stays on the versions endpoint.
    let latest = svc.bds.registry().await.ok().map(|r| r.release.latest);
    let binary_present = svc
        .config
        .bedrock_server_dir
        .join(if cfg!(windows) {
            "bedrock_server.exe"
        } else {
            "bedrock_server"
        })
        .exists();
    let install = serde_json::to_value(&*svc.bds.state.borrow()).unwrap_or(Value::Null);
    let endstone_installed = svc.endstone.installed().await;
    let endstone_binary_present =
        omnibedrock_server::endstone::endstone_binary(&svc.config).exists();
    let endstone_install =
        serde_json::to_value(&*svc.endstone.state.borrow()).unwrap_or(Value::Null);
    // Update availability compares the marker against the cached PyPI
    // registry; a stale-or-never-fetched cache just reports "unknown".
    let endstone_latest = svc.endstone.cached_registry().map(|r| r.latest);
    let endstone_update_available = match (&endstone_installed, &endstone_latest) {
        (Some(installed), Some(latest)) => installed["endstone_version"]
            .as_str()
            .is_some_and(|v| v != latest),
        _ => false,
    };
    json!({
        "backend": settings.backend.as_str(),
        "vanilla_version": settings.vanilla_version,
        "running": svc.server.snapshot().status == Status::Running,
        "installed": installed,
        "latest": latest,
        "binary_present": binary_present,
        "install": install,
        "endstone": {
            "installed": endstone_installed,
            "binary_present": endstone_binary_present,
            "install": endstone_install,
            "latest": endstone_latest,
            "update_available": endstone_update_available,
        },
    })
}
pub(super) async fn install_bds(svc: &Arc<Instance>, version: &str) -> Result<Value> {
    if svc.bds.state.borrow().phase != "idle" {
        return Err(ApiError::conflict("A BDS install is already running"));
    }
    if version != "latest" {
        let registry = svc.bds.registry().await?;
        if !bds::version_exists(&registry, version) {
            return Err(ApiError::bad(format!(
                "Version '{version}' is not in the registry"
            )));
        }
    }
    let receiver = svc.bds.install(version).await?;
    let console = svc.server.console.clone();
    tokio::spawn(async move {
        match receiver.await {
            Ok(Ok(message)) => {
                console
                    .publish(format!("[LOCAL] {message}"), "panel", false)
                    .await;
            }
            result => {
                let error = result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "installer stopped".into());
                tracing::error!(%error, "BDS install failed");
                console
                    .publish(
                        format!("[LOCAL] BDS install failed: {error}"),
                        "panel",
                        false,
                    )
                    .await;
            }
        }
    });
    Ok(success(format!(
        "Installing BDS {version}; progress appears in the console"
    )))
}
/// Make sure a vanilla BDS binary exists before starting on the vanilla
/// backend; when nothing is installed, run the download inline so a plain
/// Start just works instead of failing with a "go install it" error.
pub(super) async fn ensure_vanilla_installed(svc: &Arc<Instance>) -> Result<()> {
    let binary = svc.config.bedrock_server_dir.join(if cfg!(windows) {
        "bedrock_server.exe"
    } else {
        "bedrock_server"
    });
    if binary.exists() {
        return Ok(());
    }
    if svc.bds.state.borrow().phase != "idle" {
        return Err(ApiError::conflict(
            "A BDS install is already running; start the server again once it finishes",
        ));
    }
    let version = svc.server.settings.borrow().vanilla_version.clone();
    let requested = if version.is_empty() {
        "latest".to_owned()
    } else {
        version
    };
    svc.server
        .console
        .publish(
            format!("[LOCAL] Vanilla BDS is not installed — installing {requested} first…"),
            "panel",
            false,
        )
        .await;
    let receiver = svc
        .bds
        .install(&requested)
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    match receiver.await {
        Ok(Ok(message)) => {
            // The installer may have written its own default properties;
            // re-pin the instance's registered port (both stacks).
            omnibedrock_server::instances::enforce_instance_port(&svc.config, svc.record().port)
                .await;
            svc.server
                .console
                .publish(format!("[LOCAL] {message}"), "panel", false)
                .await;
            Ok(())
        }
        Ok(Err(e)) => Err(ApiError::conflict(format!("BDS install failed: {e}"))),
        Err(_) => Err(ApiError::conflict("BDS installer stopped")),
    }
}

/// Make sure an Endstone launch candidate exists before starting on the
/// Endstone backend. When nothing is installed, run the toolchain bootstrap
/// inline (uv + managed CPython + Endstone) so a plain Start just works.
pub(super) async fn ensure_endstone_toolchain(svc: &Arc<Instance>) -> Result<()> {
    let config = svc.config.clone();
    let ready = tokio::task::spawn_blocking(move || {
        omnibedrock_server::endstone::resolve_launch(&config).is_some()
    })
    .await
    .map_err(anyhow::Error::from)?;
    if ready {
        return Ok(());
    }
    if svc.endstone.state.borrow().phase != "idle" {
        return Err(ApiError::conflict(
            "An Endstone toolchain install is already running; start the server again once it finishes",
        ));
    }
    svc.server
        .console
        .publish(
            "[LOCAL] Endstone toolchain not found — installing uv + Python + Endstone first…"
                .to_string(),
            "panel",
            false,
        )
        .await;
    let receiver = svc
        .endstone
        .install(None)
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    match receiver.await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(ApiError::conflict(format!(
            "Endstone toolchain install failed: {e}"
        ))),
        Err(_) => Err(ApiError::conflict("Endstone toolchain installer stopped")),
    }
}
pub(super) async fn install_endstone(
    svc: &Arc<Instance>,
    version: Option<String>,
) -> Result<Value> {
    if let Some(v) = &version {
        component(v)?;
        // Same guard as the BDS installer: reject versions PyPI never
        // published instead of failing later inside uv.
        let registry = svc.endstone.registry().await?;
        if !endstone::version_exists(&registry, v) {
            return Err(ApiError::bad(format!(
                "Endstone version '{v}' is not in the release registry"
            )));
        }
    }
    if svc.endstone.state.borrow().phase != "idle" {
        return Err(ApiError::conflict(
            "An Endstone toolchain install is already running",
        ));
    }
    let receiver = svc.endstone.install(version.clone()).await?;
    let console = svc.server.console.clone();
    let requested = version.unwrap_or_else(|| "latest".into());
    tokio::spawn(async move {
        match receiver.await {
            Ok(Ok(message)) => {
                console
                    .publish(format!("[LOCAL] {message}"), "panel", false)
                    .await;
            }
            result => {
                let error = result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "installer stopped".into());
                tracing::error!(%error, "Endstone toolchain install failed");
                console
                    .publish(
                        format!("[LOCAL] Endstone install failed: {error}"),
                        "panel",
                        false,
                    )
                    .await;
            }
        }
    });
    Ok(success(format!(
        "Installing Endstone toolchain ({requested}); progress appears in the console"
    )))
}
pub(super) async fn switch_backend(svc: &Arc<Instance>, b: &Value) -> Result<Value> {
    let backend = Backend::parse(value_string(b, "backend")?)?;
    let mut settings = svc.server.settings.borrow().clone();
    if let Some(version) = b["version"].as_str() {
        component(version)?;
        let registry = svc.bds.registry().await?;
        if version != "latest" && !bds::version_exists(&registry, version) {
            return Err(ApiError::bad(format!(
                "Version '{version}' is not in the registry"
            )));
        }
        settings.vanilla_version = version.into();
    }
    // Both runtimes install on demand, so a switch never dead-ends with a
    // "go install it first" error (mirrors the plain-Start bootstrap).
    if backend == Backend::Vanilla {
        ensure_vanilla_installed(svc).await?;
    }
    if backend == Backend::Endstone && svc.record().deployment != Deployment::Docker {
        ensure_endstone_toolchain(svc).await?;
    }
    let console = svc.server.console.clone();
    console
        .publish(
            format!(
                "[LOCAL] Switching '{}' to the {} backend — worlds, allowlist and \
                 player permissions are kept (both runtimes share the server directory)",
                svc.record().name,
                backend.as_str()
            ),
            "panel",
            false,
        )
        .await;
    let was_running = svc.server.snapshot().status == Status::Running;
    let lock = svc.operations.clone().lock_owned().await;
    if was_running && let Err(e) = svc.server.action("stop").await {
        drop(lock);
        return Err(ApiError::conflict(format!(
            "Stop before switch failed: {e}"
        )));
    }
    settings.backend = backend;
    let configured = svc.server.configure(settings).await;
    let restarted = if was_running {
        svc.server.action("start").await
    } else {
        Ok(String::new())
    };
    drop(lock);
    configured.map_err(|e| ApiError::conflict(e.to_string()))?;
    restarted.map_err(|e| ApiError::conflict(e.to_string()))?;
    console
        .publish(
            format!("[LOCAL] Backend switched to {}", backend.as_str()),
            "panel",
            false,
        )
        .await;
    Ok(success(format!(
        "Backend switched to {}; worlds and player permissions are kept; {}",
        backend.as_str(),
        if was_running {
            "server restarted on the new backend"
        } else {
            "applies on next start"
        }
    )))
}
