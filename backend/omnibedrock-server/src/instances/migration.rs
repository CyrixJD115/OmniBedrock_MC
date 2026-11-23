//! Deployment migration (native ↔ systemd ↔ docker): a preflight that says
//! exactly what will happen and what blocks it, and the cleanup of the old
//! runtime once the switch is applied. The server files never move: every
//! deployment runs the same `<instance>/server` directory.

use super::{DeployCapabilities, Deployment, Instance, container_name};
use crate::server::{Backend, Status};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub level: Level,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MigrationPlan {
    pub from: &'static str,
    pub to: &'static str,
    /// False when any check is an error.
    pub ready: bool,
    pub checks: Vec<Check>,
    /// What applying the change and the next start will do, in order.
    pub steps: Vec<String>,
}

fn check(id: &'static str, level: Level, message: impl Into<String>) -> Check {
    Check {
        id,
        level,
        message: message.into(),
    }
}

/// Files under `dir` not owned by the panel's uid (a native or systemd
/// process could not rewrite them). Bounded walk; returns (count, sample).
#[cfg(unix)]
fn foreign_owned(dir: &Path) -> (u64, Option<String>) {
    use std::os::unix::fs::MetadataExt;
    let me = nix::unistd::getuid().as_raw();
    let mut count = 0u64;
    let mut sample = None;
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .flatten()
        .take(200_000)
    {
        if let Ok(meta) = entry.metadata()
            && meta.uid() != me
        {
            count += 1;
            sample.get_or_insert_with(|| entry.path().display().to_string());
        }
    }
    (count, sample)
}

#[cfg(not(unix))]
fn foreign_owned(_dir: &Path) -> (u64, Option<String>) {
    (0, None)
}

/// Whether the runtime of `deployment` is actually up for this instance,
/// independent of the panel's own status (a container or unit can outlive
/// the panel and would hold the port).
async fn runtime_active(instance: &Instance, deployment: Deployment) -> bool {
    let id = instance.record().id;
    match deployment {
        Deployment::Native => instance.server.snapshot().pid.is_some(),
        Deployment::Docker => {
            crate::docker::state(&container_name(&id)).await.as_deref() == Some("running")
        }
        Deployment::Systemd => matches!(
            crate::systemd::active_state(&id).await.as_deref(),
            Some("active" | "activating" | "reloading" | "deactivating")
        ),
    }
}

async fn image_present(image: &str) -> bool {
    tokio::process::Command::new("docker")
        .args(["image", "inspect", image])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Everything that decides whether switching `instance` to `target` is
/// safe right now, plus the steps it implies.
pub async fn preflight(
    instance: &Instance,
    target: Deployment,
    caps: &DeployCapabilities,
    autostart: bool,
) -> MigrationPlan {
    let record = instance.record();
    let from = record.deployment;
    let snapshot = instance.server.snapshot();
    let backend = instance.server.settings.borrow().backend;
    let mut checks = Vec::new();
    let mut steps = Vec::new();

    if from == target {
        checks.push(check(
            "same",
            Level::Info,
            format!("Already on {}", target.as_str()),
        ));
    }
    match snapshot.status {
        Status::Stopped | Status::Crashed => {
            checks.push(check("stopped", Level::Ok, "The server is stopped"))
        }
        other => checks.push(check(
            "stopped",
            Level::Error,
            format!("The server is {other:?}; stop it before changing how it runs").to_lowercase(),
        )),
    }
    if from != target && runtime_active(instance, from).await {
        checks.push(check(
            "old_runtime",
            Level::Error,
            format!("The {} runtime is still running outside the panel's control; stop it first or it will hold the port", from.as_str()),
        ));
    }
    if instance.backups.active() {
        checks.push(check("backup", Level::Error, "A backup is running"));
    }
    match target {
        Deployment::Native => {
            checks.push(check(
                "target",
                Level::Ok,
                "Native processes are always available",
            ));
            checks.push(check(
                "limits",
                Level::Info,
                "Memory/CPU limits become advisory (monitored, not enforced)",
            ));
        }
        Deployment::Systemd => {
            if caps.systemd {
                checks.push(check("target", Level::Ok, "systemd user manager available"));
            } else {
                checks.push(check(
                    "target",
                    Level::Error,
                    "No systemd user manager on this host",
                ));
            }
            if autostart && !caps.linger {
                checks.push(check(
                    "linger",
                    Level::Warning,
                    "Autostart is on but linger is off: the unit stops when you log out. Run `loginctl enable-linger` once.",
                ));
            }
        }
        Deployment::Docker => {
            match &caps.docker {
                Some(v) => checks.push(check("target", Level::Ok, format!("Docker {v} available"))),
                None => checks.push(check(
                    "target",
                    Level::Error,
                    "Docker is not available on this host",
                )),
            }
            if caps.docker.is_some() {
                let image = record
                    .docker
                    .clone()
                    .unwrap_or_default()
                    .image_for(backend, &instance.config);
                if image_present(&image).await {
                    checks.push(check(
                        "image",
                        Level::Ok,
                        format!("Image {image} is present"),
                    ));
                } else if image.starts_with("omnibedrock-") {
                    checks.push(check(
                        "image",
                        Level::Info,
                        format!("Image {image} will be built on first start (a few minutes)"),
                    ));
                } else {
                    checks.push(check(
                        "image",
                        Level::Error,
                        format!("Image {image} is missing; pull it first"),
                    ));
                }
            }
            if backend == Backend::Endstone {
                checks.push(check(
                    "endstone",
                    Level::Info,
                    "The container uses the Endstone build inside its image, not the host toolchain; plugins built for another Endstone version may need updating",
                ));
            }
        }
    }
    {
        let dir = instance.config.bedrock_server_dir.clone();
        let (count, sample) = tokio::task::spawn_blocking(move || foreign_owned(&dir))
            .await
            .unwrap_or((0, None));
        if count > 0 {
            checks.push(check(
                "ownership",
                Level::Error,
                format!(
                    "{count} file(s) in the server folder belong to another user (e.g. {}); the server could not update them. Fix with: sudo chown -R $(id -u):$(id -g) <server folder>",
                    sample.unwrap_or_default()
                ),
            ));
        } else {
            checks.push(check(
                "ownership",
                Level::Ok,
                "All server files belong to the panel user",
            ));
        }
    }
    if from != target {
        match from {
            Deployment::Systemd => steps.push(format!(
                "Remove the user unit {}",
                crate::systemd::unit_name(&record.id)
            )),
            Deployment::Docker => steps.push(format!(
                "Remove the stopped containers {0} and {0}-broadcaster",
                container_name(&record.id)
            )),
            Deployment::Native => {}
        }
        steps.push(match target {
            Deployment::Native => {
                "Next start launches the server as a child process of the panel".to_owned()
            }
            Deployment::Systemd => format!(
                "Next start writes and starts {} (survives panel restarts{})",
                crate::systemd::unit_name(&record.id),
                if autostart { ", starts at boot" } else { "" }
            ),
            Deployment::Docker => format!(
                "Next start runs container {} with the server folder mounted at /data",
                container_name(&record.id)
            ),
        });
        steps.push(
            "World, packs, settings, backups and broadcaster data stay where they are".to_owned(),
        );
    }
    let ready = !checks.iter().any(|c| c.level == Level::Error);
    MigrationPlan {
        from: from.as_str(),
        to: target.as_str(),
        ready,
        checks,
        steps,
    }
}

/// Remove what the old deployment left behind after a successful switch.
pub async fn retire(id: &str, from: Deployment) {
    match from {
        Deployment::Systemd => {
            if let Err(e) = crate::systemd::remove_unit(id).await {
                tracing::warn!(%e, "retiring the old systemd unit failed");
            }
        }
        Deployment::Docker => {
            crate::docker::rm(&container_name(id)).await;
            crate::docker::rm(&format!("{}-broadcaster", container_name(id))).await;
        }
        Deployment::Native => {}
    }
}
