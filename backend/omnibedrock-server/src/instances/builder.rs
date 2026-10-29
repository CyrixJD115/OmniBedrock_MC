//! Construction of a running instance from its record: the launch context,
//! the broadcaster sidecar spec, and `build_instance`, which wires up every
//! per-instance service (config, server manager, BDS/Endstone watchers,
//! broadcaster, backups) with legacy-path handling for the default instance.

use super::consts::DEFAULT_INSTANCE;
use super::helpers::container_name;
use super::instance::Instance;
use super::types::{Deployment, InstanceRecord};
use crate::server::{Console, LaunchContext, ServerManager, ServerPaths};
use crate::{backups::Backups, bds::Bds, broadcaster::Broadcaster, endstone::Endstone};
use omnibedrock_core::config::Config;
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;

pub(super) fn launch_context(record: &InstanceRecord) -> LaunchContext {
    LaunchContext {
        instance_id: record.id.clone(),
        instance_name: record.name.clone(),
        deployment: record.deployment,
        resources: record.resources,
        port: record.port,
        autostart: record.autostart,
        docker: record.docker.clone().unwrap_or_default(),
    }
}

/// The broadcaster sidecar spec for one record: a container only when the
/// instance runs on the docker backend AND the host can give the sidecar the
/// same network view as a native broadcaster (Linux host networking). Every
/// other combination keeps the native Java child process.
pub fn broadcaster_docker_spec(
    record: &InstanceRecord,
    config: &Config,
) -> Option<crate::broadcaster::DockerSpec> {
    (record.deployment == Deployment::Docker && cfg!(target_os = "linux")).then(|| {
        crate::broadcaster::DockerSpec {
            container: format!("{}-broadcaster", container_name(&record.id)),
            image: config.docker_image_broadcaster.clone(),
        }
    })
}

/// Build every service for one instance from its record. The default instance
/// keeps the legacy paths so pre-multi-instance data keeps working.
pub(super) async fn build_instance(
    config: &Arc<Config>,
    record: InstanceRecord,
) -> anyhow::Result<Arc<Instance>> {
    let (instance_config, paths) = if record.id == DEFAULT_INSTANCE {
        (
            config.clone(),
            ServerPaths {
                settings_path: config.data_dir.join("server_settings.yaml"),
                state_dir: config.data_dir.clone(),
                journal_root: config.logs_dir.join("console"),
            },
        )
    } else {
        let root = config.data_dir.join("instances").join(&record.id);
        let mut instance_config = config.as_ref().clone();
        instance_config.bedrock_server_dir = root.join("server");
        instance_config.backups_dir = config.backups_dir.join(&record.id);
        instance_config.logs_dir = config.logs_dir.join("instances").join(&record.id);
        // Own broadcaster home per instance; the jar is shared from the
        // panel-level broadcaster dir (see Broadcaster::jar_path).
        instance_config.broadcaster_dir = root.join("broadcaster");
        let instance_config = Arc::new(instance_config);
        for dir in [
            instance_config.bedrock_server_dir.join("worlds"),
            instance_config.backups_dir.clone(),
            instance_config.logs_dir.clone(),
            root.clone(),
        ] {
            tokio::fs::create_dir_all(&dir).await?;
        }
        let settings_path = root.join("server_settings.yaml");
        let paths = ServerPaths {
            settings_path: settings_path.clone(),
            state_dir: root.clone(),
            journal_root: instance_config.logs_dir.join("console"),
        };
        // Instance-scoped backup settings; the default instance keeps the
        // legacy <data_dir>/backup_settings.yaml.
        if !settings_path.exists() {
            let _ = tokio::fs::write(
                &settings_path,
                serde_yaml::to_string(&crate::server::ServerSettings::default())?.as_bytes(),
            )
            .await;
        }
        (instance_config, paths)
    };
    let token = CancellationToken::new();
    let console = Console::with_journal_root(
        paths.journal_root.clone(),
        instance_config.console_max_lines,
        instance_config.console_max_bytes,
        instance_config.console_journal_bytes,
    );
    let server = ServerManager::with_paths(
        instance_config.clone(),
        token.clone(),
        paths,
        launch_context(&record),
    )?;
    let bds = Bds::new(instance_config.clone(), console.clone(), token.clone());
    let endstone = Endstone::new(instance_config.clone(), console.clone(), token.clone());
    // The default instance's broadcaster dir IS the panel dir, so its
    // pre-existing config.yml/cache keep working untouched.
    let broadcaster = Broadcaster::new(
        instance_config.clone(),
        token.clone(),
        (record.id != DEFAULT_INSTANCE).then(|| config.broadcaster_dir.clone()),
        broadcaster_docker_spec(&record, config),
    )?;
    let backups = if record.id == DEFAULT_INSTANCE {
        Backups::new(instance_config.clone(), server.clone(), token.clone()).await?
    } else {
        Backups::with_settings_path(
            instance_config.clone(),
            server.clone(),
            token.clone(),
            instance_config
                .data_dir
                .join("instances")
                .join(&record.id)
                .join("backup_settings.yaml"),
        )
        .await?
    };
    let instance = Arc::new(Instance {
        record: RwLock::new(record),
        config: instance_config,
        server,
        bds,
        endstone,
        broadcaster,
        backups,
        token,
        operations: Arc::new(tokio::sync::Mutex::new(())),
    });
    // A systemd unit or docker container keeps running across panel
    // restarts; reconnect console and monitoring instead of reporting the
    // instance stopped.
    if matches!(
        instance
            .record
            .read()
            .expect("instance record lock")
            .deployment,
        Deployment::Systemd | Deployment::Docker
    ) {
        match instance.server.adopt_if_active().await {
            Ok(message) if message.starts_with("Adopted") => {
                tracing::info!(instance = %instance.record.read().expect("instance record lock").id, "{message}")
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(%e, "unit adoption skipped"),
        }
    }
    Ok(instance)
}
