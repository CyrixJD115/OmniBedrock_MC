//! The Endstone install/update pipeline: resolve the target release, skip
//! no-op updates, choose bootstrap vs in-place upgrade, run the uv steps,
//! and write the toolchain marker.
use crate::server::Console;
use omnibedrock_core::config::Config;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::manager::Endstone;
use super::paths::{endstone_binary, runtime_dir, venv_dir};
use super::pypi::{EndstoneRegistry, fetch_registry_cached};
use super::toolchain::{PYTHON_VERSION, UV_VERSION, ensure_uv, installed_endstone_version, run_uv};

#[allow(clippy::too_many_lines)]
pub(super) async fn run_install(
    config: &Config,
    state: &watch::Sender<crate::bds::InstallState>,
    http: &reqwest::Client,
    registry_cache: &tokio::sync::RwLock<Option<(std::time::Instant, EndstoneRegistry)>>,
    console: &Console,
    version: Option<String>,
) -> anyhow::Result<String> {
    // Resolve the target release up front: a versionless install means
    // "latest", resolved through the PyPI registry exactly like the BDS
    // updater resolves "latest" through its version registry. When the
    // registry is unreachable, fall back to letting pip pick the newest
    // release it can see rather than failing the whole install.
    let resolved: Option<String> = match &version {
        Some(v) => Some(v.clone()),
        None => match fetch_registry_cached(http, registry_cache).await {
            Ok(registry) => Some(registry.latest),
            Err(e) => {
                console
                    .publish(
                        format!("[LOCAL] Endstone: {e:#} — pip will pick the newest release"),
                        "panel",
                        false,
                    )
                    .await;
                None
            }
        },
    };
    let spec = match &resolved {
        Some(v) => format!("endstone=={v}"),
        None => "endstone".to_string(),
    };

    let installed_version = read_marker_version(config).await;
    let marker_present = tokio::fs::try_exists(Endstone::installed_marker(config))
        .await
        .unwrap_or(false);
    // A versionless install that is already at the resolved latest is a
    // no-op — this is the update check. Without a resolved version (registry
    // down) keep the old "existing toolchain" skip so routine starts never
    // reinstall just because PyPI was briefly unreachable.
    if version.is_none()
        && match (&resolved, &installed_version) {
            (Some(latest), Some(installed)) => installed == latest,
            (None, _) => {
                marker_present
                    && tokio::fs::try_exists(endstone_binary(config))
                        .await
                        .unwrap_or(false)
            }
            (Some(_), None) => false,
        }
    {
        let summary = tokio::fs::read_to_string(Endstone::installed_marker(config))
            .await
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|marker| {
                format!(
                    "endstone {}, CPython {}, uv {}",
                    marker["endstone_version"].as_str().unwrap_or("?"),
                    marker["python_version"].as_str().unwrap_or(PYTHON_VERSION),
                    marker["uv_version"].as_str().unwrap_or("?")
                )
            })
            .unwrap_or_else(|| "existing toolchain".to_string());
        let already_latest = matches!((&resolved, &installed_version),
            (Some(l), Some(i)) if i == l);
        let message = if already_latest {
            format!("Endstone toolchain already at latest ({summary}) — nothing to update")
        } else {
            format!("Endstone toolchain already installed ({summary}) — skipping reinstall")
        };
        console
            .publish(format!("[LOCAL] {message}"), "panel", false)
            .await;
        return Ok(message);
    }

    // The versionless "update to latest" upgrades the package inside the
    // existing venv when it still holds a working `endstone` executable;
    // explicit version installs keep the old full-recreate semantics (that is
    // the repair path after a broken venv). uv and the managed CPython only
    // need provisioning on the bootstrap path.
    let venv = venv_dir(config);
    let venv_python = venv.join(if cfg!(windows) {
        "Scripts\\python.exe"
    } else {
        "bin/python"
    });
    let in_place = version.is_none()
        && tokio::fs::try_exists(endstone_binary(config))
            .await
            .unwrap_or(false);

    state.send_modify(|s| {
        s.phase = "uv".into();
        s.version = resolved.clone();
        s.detail = None;
        s.progress = None;
    });
    let uv = ensure_uv(config, http, state, console).await?;

    if in_place {
        state.send_modify(|s| {
            s.phase = "endstone".into();
            s.progress = None;
        });
        console
            .publish(
                format!(
                    "[LOCAL] Endstone: upgrading venv at {} ({})",
                    venv.display(),
                    spec
                ),
                "panel",
                false,
            )
            .await;
    } else {
        state.send_modify(|s| {
            s.phase = "python".into();
            s.progress = None;
        });
        console
            .publish(
                format!("[LOCAL] Endstone: installing managed CPython {PYTHON_VERSION}"),
                "panel",
                false,
            )
            .await;
        run_uv(
            config,
            &uv,
            &["python", "install", PYTHON_VERSION],
            "python install",
        )
        .await?;

        state.send_modify(|s| {
            s.phase = "venv".into();
            s.progress = None;
        });
        console
            .publish(
                format!(
                    "[LOCAL] Endstone: creating virtualenv at {}",
                    venv.display()
                ),
                "panel",
                false,
            )
            .await;
        run_uv(
            config,
            &uv,
            &[
                "venv",
                venv.to_string_lossy().as_ref(),
                "--python",
                PYTHON_VERSION,
                "--clear",
            ],
            "create venv",
        )
        .await?;

        state.send_modify(|s| {
            s.phase = "endstone".into();
            s.progress = None;
        });
        console
            .publish(
                format!("[LOCAL] Endstone: installing {spec} (prebuilt wheel)"),
                "panel",
                false,
            )
            .await;
    }
    run_uv(
        config,
        &uv,
        &[
            "pip",
            "install",
            "--python",
            venv_python.to_string_lossy().as_ref(),
            "--upgrade",
            &spec,
        ],
        "pip install endstone",
    )
    .await?;
    let installed = installed_endstone_version(config, &uv, &venv_python).await;

    // Drop pre-relocation toolchain pieces if the runtime moved to a
    // space-free location; they can never launch correctly there.
    let preferred = config.data_dir.join("tools");
    if runtime_dir(config) != preferred {
        for legacy in ["endstone", "python", "uv-cache"] {
            let dir = preferred.join(legacy);
            if tokio::fs::try_exists(&dir).await.unwrap_or(false) {
                let _ = tokio::fs::remove_dir_all(&dir).await;
            }
        }
    }

    let mut marker = json!({
        "uv_version": UV_VERSION,
        "python_version": PYTHON_VERSION,
        "endstone_version": installed,
        "binary": endstone_binary(config),
        "installed_at": crate::server::now(),
    });
    if let Some(previous) = installed_version
        && Some(&previous) != installed.as_ref()
    {
        marker["previous_version"] = json!(previous);
    }
    if let Some(resolved) = &resolved {
        marker["requested"] = json!(resolved);
    }
    omnibedrock_core::files::atomic_write(
        &Endstone::installed_marker(config),
        serde_json::to_vec(&marker)?.as_slice(),
    )
    .await?;
    let message = format!(
        "Endstone toolchain ready (endstone {}, CPython {PYTHON_VERSION}, uv {UV_VERSION})",
        installed.as_deref().unwrap_or("unknown")
    );
    console
        .publish(format!("[LOCAL] {message}"), "panel", false)
        .await;
    Ok(message)
}

/// The `endstone_version` recorded in the toolchain marker, if any.
pub(crate) async fn read_marker_version(config: &Config) -> Option<String> {
    let text = tokio::fs::read_to_string(Endstone::installed_marker(config))
        .await
        .ok()?;
    let marker: Value = serde_json::from_str(&text).ok()?;
    marker["endstone_version"].as_str().map(str::to_string)
}
