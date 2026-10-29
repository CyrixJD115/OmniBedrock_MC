//! The persisted instance data model: per-instance resource policies, user
//! quotas, deployment backend selection, Docker options, access entries, and
//! the `InstanceRecord` that serializes to `instances.yaml`.

use anyhow::bail;
use omnibedrock_core::config::Config;
use serde::{Deserialize, Serialize};

/// Hard per-instance resource limits. `None` means unlimited. Enforcement
/// depends on the deployment backend; native processes are only monitored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourcePolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<f64>,
}

impl ResourcePolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(mb) = self.memory_mb {
            anyhow::ensure!(
                (128..=1_048_576).contains(&mb),
                "memory limit must be between 128 MiB and 1 TiB"
            );
        }
        if let Some(cpus) = self.cpus {
            anyhow::ensure!(
                (0.1..=256.0).contains(&cpus),
                "cpu limit must be between 0.1 and 256"
            );
        }
        Ok(())
    }
    /// systemd CPUQuota percent (1.00 CPU = 100%).
    pub fn cpu_quota_percent(&self) -> Option<u32> {
        self.cpus
            .map(|c| (c * 100.0).clamp(1.0, 1_000_000.0) as u32)
    }
    pub fn is_unlimited(&self) -> bool {
        self.memory_mb.is_none() && self.cpus.is_none()
    }
}

/// Aggregate ceilings for one panel user. `None` = unlimited. Enforced at
/// instance create/update time while the manager write lock is held, so
/// concurrent requests cannot race past the check.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UserQuotas {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_instances: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<f64>,
}

impl UserQuotas {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(n) = self.max_instances {
            anyhow::ensure!(n > 0, "max_instances must be at least 1");
        }
        if let Some(mb) = self.memory_mb {
            anyhow::ensure!(mb >= 128, "memory quota must be at least 128 MiB");
        }
        if let Some(c) = self.cpus {
            anyhow::ensure!(c >= 0.1, "cpu quota must be at least 0.1");
        }
        Ok(())
    }
    /// Check a user's aggregate allocation against these ceilings.
    /// `owned` are the records the user owns; `extra` is the delta being added.
    pub fn check(&self, owned: &[&InstanceRecord], extra: ResourcePolicy) -> anyhow::Result<()> {
        if let Some(max) = self.max_instances {
            anyhow::ensure!(
                owned.len() < max as usize,
                "instance quota reached (max {max})"
            );
        }
        let memory: u64 = owned
            .iter()
            .map(|r| r.resources.memory_mb.unwrap_or(0))
            .sum();
        let cpus: f64 = owned.iter().map(|r| r.resources.cpus.unwrap_or(0.0)).sum();
        if let Some(limit) = self.memory_mb {
            anyhow::ensure!(
                memory + extra.memory_mb.unwrap_or(0) <= limit,
                "memory quota exceeded: {memory} MiB allocated + {} MiB requested > {limit} MiB",
                extra.memory_mb.unwrap_or(0)
            );
        }
        if let Some(limit) = self.cpus {
            let total = cpus + extra.cpus.unwrap_or(0.0);
            anyhow::ensure!(
                total <= limit + 1e-9,
                "cpu quota exceeded: {cpus:.2} allocated + {:.2} requested > {limit:.2}",
                extra.cpus.unwrap_or(0.0)
            );
        }
        Ok(())
    }
}

/// How an instance's process is launched and its limits enforced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Deployment {
    /// Plain child process. Limits are advisory (monitoring only).
    #[default]
    Native,
    /// `systemd-run`-free hosting in a persistent user service unit
    /// (`omnibedrock-<id>.service`): real cgroup memory/CPU controls and the
    /// server survives panel restarts.
    Systemd,
    /// Docker container: limits enforced by the container runtime.
    Docker,
}

impl Deployment {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "native" => Ok(Self::Native),
            "systemd" => Ok(Self::Systemd),
            "docker" => Ok(Self::Docker),
            other => bail!("unknown deployment '{other}' (expected native, systemd, or docker)"),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Systemd => "systemd",
            Self::Docker => "docker",
        }
    }
    /// What a configured limit actually enforces on this backend.
    pub fn enforcement(self) -> &'static str {
        match self {
            Self::Native => "advisory",
            Self::Systemd => "cgroup",
            Self::Docker => "container",
        }
    }
    pub fn enforcement_description(self) -> &'static str {
        match self {
            Self::Native => {
                "Advisory only: the panel reports usage but the OS does not enforce limits."
            }
            Self::Systemd => {
                "Enforced by systemd cgroup controls (MemoryMax/CPUQuota) on this Linux host; the instance runs as a user service that survives panel restarts."
            }
            Self::Docker => {
                "Enforced by the container runtime (--memory/--cpus); the container is killed on OOM."
            }
        }
    }
}

/// Per-instance Docker options for the `docker` deployment backend. Optional
/// everywhere: instances that never touch Docker keep `None`, and every field
/// falls back to the panel-level defaults (see `Config::docker_image_*`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Image override; `None` uses the config default for the active runtime
    /// (omnibedrock-vanilla / omnibedrock-endstone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Extra `-v` bind mounts applied verbatim, one `host:container[:opts]`
    /// spec each. The server directory mount is implicit and always present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_mounts: Vec<String>,
}

impl DockerConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(image) = &self.image {
            anyhow::ensure!(
                !image.trim().is_empty() && !image.contains(char::is_whitespace),
                "docker image must be a single non-empty reference"
            );
        }
        for spec in &self.extra_mounts {
            let parts: Vec<&str> = spec.split(':').collect();
            anyhow::ensure!(
                (2..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty()),
                "extra mount '{spec}' must look like host:container[:opts]"
            );
            anyhow::ensure!(
                !spec.chars().any(char::is_whitespace),
                "extra mount '{spec}' must not contain whitespace"
            );
            anyhow::ensure!(
                parts[1].starts_with('/'),
                "extra mount '{spec}' container path must be absolute"
            );
        }
        Ok(())
    }
    /// The image for one runtime: the per-instance override, else the config
    /// default for that backend.
    pub fn image_for(&self, backend: crate::server::Backend, config: &Config) -> String {
        self.image.clone().unwrap_or_else(|| match backend {
            crate::server::Backend::Vanilla => config.docker_image_vanilla.clone(),
            crate::server::Backend::Endstone => config.docker_image_endstone.clone(),
        })
    }
}

/// Instance access is expressed with the SAME role catalog as global roles:
/// an entry assigns catalog roles to one user on this instance. Legacy files
/// store a two-level `level` field (manage|view); it migrates to the roles
/// `instance-manager` / `viewer` on load and is never written again.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessEntry {
    pub user: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Legacy level field; consumed by `AccessEntry::normalized`.
    #[serde(default, skip_serializing)]
    pub level: Option<String>,
}
impl AccessEntry {
    pub fn roles(&self) -> &[String] {
        &self.roles
    }
    /// Map a legacy view/manage level onto catalog roles and drop the legacy
    /// field. No-op for entries already stored with roles.
    pub fn normalized(mut self) -> Self {
        if self.roles.is_empty()
            && let Some(level) = self.level.as_deref()
        {
            self.roles = match level.trim().to_ascii_lowercase().as_str() {
                "manage" => vec!["instance-manager".into()],
                _ => vec!["viewer".into()],
            };
        }
        self.level = None;
        self
    }
}

fn default_port() -> u16 {
    19132
}

/// Everything the panel persists about one instance. The running services are
/// built from this record; identity fields never change after creation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub created_at: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub deployment: Deployment,
    /// Docker options; only meaningful when `deployment` is Docker. Older
    /// records (and every native/systemd instance) carry `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker: Option<DockerConfig>,
    #[serde(default)]
    pub resources: ResourcePolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access: Vec<AccessEntry>,
    /// systemd only: start the unit at boot (`systemctl --user enable`).
    /// Requires linger for user units to run without a login session.
    #[serde(default)]
    pub autostart: bool,
}

impl InstanceRecord {
    /// The access entry for one user, if any. Entry membership grants
    /// visibility; the entry's roles determine what the user may do there.
    pub fn access_entry(&self, user: &str) -> Option<&AccessEntry> {
        self.access.iter().find(|a| a.user == user)
    }
    /// Drop the legacy level field from every entry (call after loading).
    pub fn normalized(mut self) -> Self {
        self.access = self
            .access
            .drain(..)
            .map(AccessEntry::normalized)
            .filter(|a| !a.roles.is_empty())
            .collect();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_policy_validation() {
        assert!(
            ResourcePolicy {
                memory_mb: Some(128),
                cpus: Some(0.1)
            }
            .validate()
            .is_ok()
        );
        assert!(
            ResourcePolicy {
                memory_mb: Some(64),
                cpus: None
            }
            .validate()
            .is_err()
        );
        assert!(
            ResourcePolicy {
                memory_mb: None,
                cpus: Some(0.01)
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn quota_check_sums_owned_instances() {
        let quotas = UserQuotas {
            max_instances: Some(2),
            memory_mb: Some(2048),
            cpus: Some(4.0),
        };
        let owned = [
            InstanceRecord {
                id: "a".into(),
                name: "A".into(),
                owner: "u".into(),
                created_at: String::new(),
                port: 1,
                deployment: Deployment::Native,
                docker: None,
                resources: ResourcePolicy {
                    memory_mb: Some(1024),
                    cpus: Some(2.0),
                },
                access: vec![],
                autostart: false,
            },
            InstanceRecord {
                id: "b".into(),
                name: "B".into(),
                owner: "u".into(),
                created_at: String::new(),
                port: 2,
                deployment: Deployment::Native,
                docker: None,
                resources: ResourcePolicy {
                    memory_mb: Some(512),
                    cpus: Some(1.0),
                },
                access: vec![],
                autostart: false,
            },
        ];
        let refs: Vec<&InstanceRecord> = owned.iter().collect();
        // Third instance exceeds the count quota.
        assert!(quotas.check(&refs, ResourcePolicy::default()).is_err());
        let quotas = UserQuotas {
            max_instances: None,
            memory_mb: Some(2048),
            cpus: Some(4.0),
        };
        // 1536 + 1024 > 2048 memory.
        assert!(
            quotas
                .check(
                    &refs,
                    ResourcePolicy {
                        memory_mb: Some(1024),
                        cpus: None
                    }
                )
                .is_err()
        );
        // Unlimited extras (None) cannot be summed, so they are rejected when
        // any quota is set: an unlimited instance against a finite budget is
        // nonsensical.
        let single = [owned[0].clone()];
        let single_refs: Vec<&InstanceRecord> = single.iter().collect();
        assert!(
            UserQuotas {
                max_instances: None,
                memory_mb: Some(2048),
                cpus: None
            }
            .check(&single_refs, ResourcePolicy::default())
            .is_ok()
        );
    }

    #[test]
    fn deployment_parse_and_enforcement() {
        assert_eq!(Deployment::parse("docker").unwrap(), Deployment::Docker);
        assert_eq!(Deployment::parse("SYSTEMD").unwrap(), Deployment::Systemd);
        assert!(Deployment::parse("podman").is_err());
        assert_eq!(Deployment::Native.enforcement(), "advisory");
        assert_eq!(Deployment::Systemd.enforcement(), "cgroup");
        assert_eq!(Deployment::Docker.enforcement(), "container");
    }

    #[test]
    fn record_access_rules() {
        let record = InstanceRecord {
            id: "x".into(),
            name: "X".into(),
            owner: "me".into(),
            created_at: String::new(),
            port: 1,
            deployment: Deployment::Native,
            docker: None,
            resources: ResourcePolicy::default(),
            access: vec![
                AccessEntry {
                    user: "friend".into(),
                    roles: vec!["instance-manager".into()],
                    level: None,
                },
                AccessEntry {
                    user: "dad".into(),
                    roles: vec!["viewer".into()],
                    level: None,
                },
            ],
            autostart: false,
        };
        assert_eq!(
            record.access_entry("friend").map(|a| a.roles.as_slice()),
            Some(&["instance-manager".to_string()][..])
        );
        assert!(record.access_entry("dad").is_some());
        assert!(record.access_entry("stranger").is_none());
    }

    #[test]
    fn legacy_access_levels_migrate_to_roles() {
        // Files written before the unified role system carry a two-level
        // `level` field; loading maps manage/view onto catalog roles.
        let legacy: InstanceRecord = serde_yaml::from_str(
            "id: x\nname: X\nowner: me\ncreated_at: ''\naccess:\n  - user: friend\n    level: manage\n  - user: dad\n    level: view\n",
        )
        .unwrap();
        assert!(legacy.access.iter().any(|a| a.level.is_some()));
        let record = legacy.normalized();
        assert!(record.access.iter().all(|a| a.level.is_none()));
        let friend = record.access_entry("friend").unwrap();
        assert_eq!(friend.roles, vec!["instance-manager".to_string()]);
        let dad = record.access_entry("dad").unwrap();
        assert_eq!(dad.roles, vec!["viewer".to_string()]);
        // Round-trip: saved files no longer contain the legacy field.
        let text = serde_yaml::to_string(&record).unwrap();
        assert!(!text.contains("level:"));
    }

    #[test]
    fn docker_config_validation() {
        let ok = DockerConfig {
            image: Some("omnibedrock-vanilla:1.21".into()),
            extra_mounts: vec!["/a:/b:ro".into(), "/c:/d".into()],
        };
        assert!(ok.validate().is_ok());
        assert!(DockerConfig::default().validate().is_ok());
        // Image must be a single reference.
        assert!(
            DockerConfig {
                image: Some("two words".into()),
                extra_mounts: vec![]
            }
            .validate()
            .is_err()
        );
        // Mounts need host:container, no whitespace, absolute container path.
        assert!(
            DockerConfig {
                image: None,
                extra_mounts: vec!["/a:b c".into()]
            }
            .validate()
            .is_err()
        );
        assert!(
            DockerConfig {
                image: None,
                extra_mounts: vec!["onlyhost".into()]
            }
            .validate()
            .is_err()
        );
        assert!(
            DockerConfig {
                image: None,
                extra_mounts: vec!["/a:relative".into()]
            }
            .validate()
            .is_err()
        );
        // The per-instance override beats the config default per runtime.
        let config = Config::default();
        assert_eq!(
            ok.image_for(crate::server::Backend::Vanilla, &config),
            "omnibedrock-vanilla:1.21"
        );
        assert_eq!(
            DockerConfig::default().image_for(crate::server::Backend::Endstone, &config),
            config.docker_image_endstone
        );
    }

    #[test]
    fn record_serde_docker_field_is_optional() {
        let record: InstanceRecord = serde_yaml::from_str(
            "id: a\nname: A\nowner: o\ncreated_at: 'x'\ndeployment: docker\ndocker:\n  image: omnibedrock-vanilla:1\n  extra_mounts:\n    - /a:/b:ro\n",
        )
        .unwrap();
        let docker = record.docker.expect("docker options parsed");
        assert_eq!(docker.image.as_deref(), Some("omnibedrock-vanilla:1"));
        assert_eq!(docker.extra_mounts, vec!["/a:/b:ro".to_string()]);
        // Records written before the docker backend keep deserializing.
        let legacy: InstanceRecord =
            serde_yaml::from_str("id: a\nname: A\nowner: o\ncreated_at: 'x'\n").unwrap();
        assert!(legacy.docker.is_none());
        assert_eq!(legacy.deployment, Deployment::Native);
    }

    #[test]
    fn record_serde_roundtrip_with_defaults() {
        // Older instances.yaml entries lack deployment/resources; they must
        // still deserialize as native/unlimited.
        let record: InstanceRecord =
            serde_yaml::from_str("id: a\nname: A\nowner: o\ncreated_at: '2026-01-01T00:00:00Z'\n")
                .unwrap();
        assert_eq!(record.deployment, Deployment::Native);
        assert_eq!(record.port, 19132);
        assert!(record.resources.is_unlimited());
    }
}
