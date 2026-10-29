//! Instance CRUD handlers: create, update, resource quotas, Docker options,
//! and per-instance usage summaries.
use crate::auth::{INSTANCE_PERMISSIONS, User};
use omnibedrock_core::error::{ApiError, Result};
use omnibedrock_server::{
    instances::{AccessEntry, Deployment, DockerConfig, Instance, InstanceRecord, ResourcePolicy},
    server::{Backend, ServerSettings},
};
use serde_json::{Value, json};
use std::sync::Arc;

use super::helpers::{optional_string, value_string};
use super::state::AppState;

pub(super) fn usage_json(metrics: &Value) -> Value {
    json!({
        "cpu_percent": metrics["cpu_percent"].as_f64().unwrap_or(0.0),
        "memory_mb": metrics["memory_mb"].as_f64().unwrap_or(0.0),
        "disk_mb": metrics["disk_mb"].as_f64().unwrap_or(0.0),
        "usage_source": metrics["usage_source"].as_str().unwrap_or("unknown"),
    })
}

/// The optional per-instance Docker options from a request body. Absent or
/// null clears them (the config defaults then apply at launch).
///
/// `extra_mounts` are arbitrary host bind mounts — `-v /:/host:rw` reads the
/// panel's own data dir from inside the container — so they require BOTH the
/// operator opt-in (`DOCKER_ALLOW_HOST_MOUNTS`) and the panel-owner role.
async fn docker_from_body(s: &AppState, u: &User, b: &Value) -> Result<Option<DockerConfig>> {
    match b.get("docker") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let config = DockerConfig {
                image: v["image"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from),
                extra_mounts: v["extra_mounts"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| m.as_str().map(str::trim))
                            .filter(|m| !m.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            if !config.extra_mounts.is_empty() {
                if !s.config.docker_allow_host_mounts {
                    return Err(ApiError::forbidden(
                        "Docker bind mounts are disabled on this host; set \
                         DOCKER_ALLOW_HOST_MOUNTS=true to allow them",
                    ));
                }
                if !s.auth.is_owner(u).await {
                    return Err(ApiError::forbidden(
                        "Only the panel owner may configure Docker bind mounts",
                    ));
                }
            }
            config
                .validate()
                .map_err(|e| ApiError::bad(e.to_string()))?;
            Ok(Some(config))
        }
    }
}

/// Create an instance after checking quotas, port availability, and the
/// deployment backend's availability on this host.
pub(super) async fn create_instance(s: &AppState, u: &User, b: &Value) -> Result<Value> {
    let name = value_string(b, "name")?.trim().to_owned();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad("name must be 1..64 characters"));
    }
    omnibedrock_core::files::component(&name).map_err(|e| ApiError::bad(e.message))?;
    let backend = Backend::parse(value_string(b, "backend")?)?;
    let deployment = Deployment::parse(optional_string(b, "deployment", "native"))?;
    let resources = ResourcePolicy {
        memory_mb: b.get("memory_mb").and_then(Value::as_u64),
        cpus: b.get("cpus").and_then(Value::as_f64),
    };
    resources
        .validate()
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let port = match b.get("port") {
        None | Some(Value::Null) => 0u16,
        Some(v) => v
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| ApiError::bad("'port' must be a UDP port number (0 = auto)"))?,
    };
    let id = omnibedrock_server::instances::slug(&name);
    if !omnibedrock_server::instances::valid_id(&id) {
        return Err(ApiError::bad(format!(
            "could not derive a valid instance id from '{name}'"
        )));
    }
    let record = InstanceRecord {
        id,
        name,
        owner: u.username.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        port,
        deployment,
        docker: docker_from_body(s, u, b).await?,
        resources,
        access: vec![],
        autostart: b["autostart"].as_bool().unwrap_or(false),
    };
    // Quotas apply to everyone except the panel owner, who sets them.
    let quotas = u.quotas.clone();
    let is_owner_role = s.auth.is_owner(u).await;
    let username = u.username.clone();
    let extra = record.resources;
    let instance = s
        .instances
        .create(record, backend, move |records| {
            if is_owner_role {
                return Ok(());
            }
            let owned: Vec<&InstanceRecord> =
                records.iter().filter(|r| r.owner == username).collect();
            quotas.check(&owned, extra)
        })
        .await
        .map_err(|e| ApiError::bad(e.to_string()))?;
    // Seed the instance's runtime settings (backend + version) now that the
    // per-instance settings file exists.
    let mut settings = ServerSettings {
        backend,
        ..ServerSettings::default()
    };
    if let Some(version) = b["vanilla_version"].as_str() {
        omnibedrock_core::files::component(version).map_err(|e| ApiError::bad(e.message))?;
        settings.vanilla_version = version.into();
    }
    if let Err(e) = instance.server.configure(settings).await {
        return Err(ApiError::conflict(format!(
            "instance created but saving its settings failed: {e}"
        )));
    }
    let mut item = instance.summary();
    item["usage"] = usage_json(&Value::Null);
    Ok(item)
}

/// Update an instance's editable fields. Deployment, resources, and port
/// changes require the instance to be stopped (enforced by the manager).
pub(super) async fn update_instance(
    s: &AppState,
    u: &User,
    instance: &Arc<Instance>,
    b: &Value,
) -> Result<Value> {
    let record = instance.record();
    let actor_is_owner = s.auth.is_owner(u).await;
    let is_instance_owner = record.owner == u.username;
    // Resource/limit changes are a granular permission (INSTANCES_LIMITS,
    // grantable globally or through an instance share) on top of the owner
    // paths; the quota check below still runs for everyone. Access-list
    // management additionally accepts INSTANCES_SHARE.
    let can_edit_limits = actor_is_owner
        || is_instance_owner
        || s.auth.require(u, "INSTANCES_LIMITS").await.is_ok()
        || s.auth
            .instance_permissions(u, &record)
            .await
            .iter()
            .any(|p| p == "INSTANCES_LIMITS");
    let can_edit_access = can_edit_limits || s.auth.require(u, "INSTANCES_SHARE").await.is_ok();
    let touches_limits = ["name", "port", "deployment", "docker", "memory_mb", "cpus"]
        .iter()
        .any(|k| b.get(*k).is_some());
    if b.get("access").is_some() && !can_edit_access {
        return Err(ApiError::forbidden(
            "Only the instance owner, a panel owner, or a holder of INSTANCES_SHARE may manage instance access",
        ));
    }
    if touches_limits && !can_edit_limits {
        return Err(ApiError::forbidden(
            "Only the instance owner, a panel owner, or a holder of INSTANCES_LIMITS may change its resources",
        ));
    }
    if !touches_limits && b.get("access").is_none() {
        return Err(ApiError::bad("Nothing to update"));
    }
    let mut next = record.clone();
    if let Some(name) = b["name"].as_str() {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(ApiError::bad("name must be 1..64 characters"));
        }
        omnibedrock_core::files::component(name).map_err(|e| ApiError::bad(e.message))?;
        next.name = name.into();
    }
    if let Some(port) = b["port"].as_u64() {
        let port =
            u16::try_from(port).map_err(|_| ApiError::bad("'port' must be a UDP port number"))?;
        next.port = port;
    }
    if let Some(deployment) = b["deployment"].as_str() {
        next.deployment =
            Deployment::parse(deployment).map_err(|e| ApiError::bad(e.to_string()))?;
    }
    if b.get("docker").is_some() {
        // Replace the whole object (or clear it with null); partial patches
        // would silently keep stale mounts around.
        next.docker = docker_from_body(s, u, b).await?;
    }
    if let Some(autostart) = b["autostart"].as_bool() {
        if autostart && next.deployment != Deployment::Systemd {
            return Err(ApiError::bad(
                "autostart applies to systemd instances only (it enables the user unit)",
            ));
        }
        next.autostart = autostart;
    }
    if b.get("memory_mb").is_some() {
        next.resources.memory_mb = b["memory_mb"].as_u64();
    }
    if b.get("cpus").is_some() {
        next.resources.cpus = b["cpus"].as_f64();
    }
    if let Some(access) = b["access"].as_array() {
        let mut entries = Vec::new();
        for entry in access {
            let user = entry["user"]
                .as_str()
                .ok_or_else(|| ApiError::bad("access entries need a user"))?;
            omnibedrock_core::files::component(user).map_err(|e| ApiError::bad(e.message))?;
            // Roles reference the shared catalog; the legacy two-level field is
            // still accepted from old clients and maps the same way as files.
            let mut roles: Vec<String> = match entry.get("roles") {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(Value::String(role)) => vec![role.clone()],
                _ => match entry["level"].as_str().unwrap_or("view") {
                    "manage" => vec!["instance-manager".into()],
                    _ => vec!["viewer".into()],
                },
            };
            roles.sort();
            roles.dedup();
            if roles.is_empty() {
                continue;
            }
            // Delegation guard: without the owner sentinel you cannot grant an
            // instance capability you do not hold on this very instance. Only
            // instance-scoped permissions are effective here, so only those
            // are checked.
            if !actor_is_owner {
                let granted: Vec<String> = s
                    .auth
                    .roles()
                    .await
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter(|r| roles.contains(&r["name"].as_str().unwrap_or_default().to_string()))
                    .flat_map(|r| r["permissions"].as_array().cloned().unwrap_or_default())
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .filter(|p| INSTANCE_PERMISSIONS.contains(&p.as_str()))
                    .collect();
                let held = s.auth.instance_permissions(u, &record).await;
                if let Some(missing) = granted.iter().find(|p| !held.contains(p)) {
                    return Err(ApiError::forbidden(format!(
                        "You cannot grant {perm} on this instance — you do not hold it yourself",
                        perm = missing
                    )));
                }
            }
            entries.push(AccessEntry {
                user: user.into(),
                roles,
                level: None,
            });
        }
        next.access = entries;
    }
    next.resources
        .validate()
        .map_err(|e| ApiError::bad(e.to_string()))?;
    // A resource increase must respect the instance owner's quota (panel
    // owner exempt); the check runs inside the manager's write lock.
    let quotas = s.auth.user_quotas(&next.owner).await.unwrap_or_default();
    let is_panel_owner = actor_is_owner;
    let owner = next.owner.clone();
    let target_id = next.id.clone();
    s.instances
        .configure(&record.id, next, move |records| {
            if is_panel_owner {
                return Ok(());
            }
            let memory: u64 = records
                .iter()
                .filter(|r| r.owner == owner)
                .map(|r| r.resources.memory_mb.unwrap_or(0))
                .sum();
            let cpus: f64 = records
                .iter()
                .filter(|r| r.owner == owner)
                .map(|r| r.resources.cpus.unwrap_or(0.0))
                .sum();
            if let Some(limit) = quotas.memory_mb
                && memory > limit
            {
                return Err(anyhow::anyhow!(format!(
                    "memory quota exceeded: {memory} MiB allocated > {limit} MiB"
                )));
            }
            if let Some(limit) = quotas.cpus
                && cpus > limit + 1e-9
            {
                return Err(anyhow::anyhow!(format!(
                    "cpu quota exceeded: {cpus:.2} allocated > {limit:.2}"
                )));
            }
            Ok(())
        })
        .await
        .map_err(|e| ApiError::forbidden(e.to_string()))?;
    let instance = s
        .instances
        .get(&target_id)
        .await
        .ok_or_else(|| ApiError::missing("Instance vanished"))?;
    let mut item = instance.summary();
    item["usage"] = usage_json(&s.instance_metrics(&target_id));
    Ok(item)
}
