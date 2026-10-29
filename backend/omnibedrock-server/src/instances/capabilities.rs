//! Deployment capability probing: discovers once at boot which backends
//! (Docker, systemd user manager, linger) this host can actually use, so
//! deployment choices can only reference backends that work.

use serde::Serialize;

/// Probed once at boot and reported to the UI so deployment choices can only
/// reference backends that actually work on this host.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeployCapabilities {
    pub docker: Option<String>,
    /// Host OS as reported to the UI ("linux" | "windows" | "macos"); the
    /// instance dialog preselects the deployment default for this platform.
    pub platform: String,
    /// A systemd user manager is reachable (user-service instances can run).
    pub systemd: bool,
    /// Linger is on: user units keep running (and start at boot) without an
    /// active login session. Required for systemd autostart to mean anything.
    pub linger: bool,
}

/// Probe which deployment backends this host can actually use.
pub(super) async fn probe_capabilities() -> DeployCapabilities {
    let docker = probe_docker().await;
    let systemd = if cfg!(unix) {
        crate::systemd::user_manager_available().await
    } else {
        false
    };
    let linger = systemd && crate::systemd::linger_enabled().await;
    DeployCapabilities {
        docker,
        systemd,
        linger,
        platform: std::env::consts::OS.into(),
    }
}

async fn probe_docker() -> Option<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("docker")
            .arg("version")
            .arg("--format")
            .arg("{{.Server.Version}}")
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
