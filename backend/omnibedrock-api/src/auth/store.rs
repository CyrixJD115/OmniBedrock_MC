//! The `Auth` service: its shared state, configuration-backed construction
//! (store loading, default-account bootstrap, JWT secret and panel settings
//! discovery) and atomic persistence of the user and role stores.

use anyhow::Context as _;
use omnibedrock_core::{config::Config, files::atomic_write};
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

use super::consts::DEFAULT_PANEL_SETTINGS;
use super::password::hash_password;
use super::seed::seed_roles;
use super::types::{Store, User, timestamp};

#[derive(Clone)]
pub struct Auth {
    pub(super) store: Arc<Mutex<Store>>,
    pub(super) config: Arc<Config>,
    pub(super) key: Arc<SecretString>,
    pub(super) work: Arc<Semaphore>,
    pub(super) dummy_hash: Arc<String>,
    /// When false, every request is served as the owner account without a token.
    pub(super) login_enabled: Arc<std::sync::atomic::AtomicBool>,
    /// True until the seeded default password is changed.
    pub(super) default_credentials: Arc<std::sync::atomic::AtomicBool>,
    /// CurseForge API key saved from the Settings page; overrides the .env value.
    pub(super) curseforge_key: Arc<std::sync::RwLock<Option<String>>>,
    /// Per-account+source login backoff (see `throttle`).
    pub(super) throttle: Arc<super::throttle::LoginThrottle>,
}

async fn load_yaml<T: serde::de::DeserializeOwned + Default>(
    path: &std::path::Path,
) -> anyhow::Result<T> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => Ok(serde_yaml::from_str(&s)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

impl Auth {
    pub async fn new(config: Arc<Config>) -> anyhow::Result<Self> {
        let mut store = Store {
            users: load_yaml(&config.data_dir.join("users.yaml")).await?,
            roles: load_yaml(&config.data_dir.join("roles.yaml")).await?,
        };
        // Legacy single-role accounts migrate to the multi-role list.
        for user in store.users.values_mut() {
            if user.roles.is_empty() && !user.role.is_empty() {
                user.roles = vec![user.role.clone()];
            }
            if user.roles.is_empty() {
                user.roles = vec!["viewer".into()];
            }
            user.set_roles(user.roles.clone());
        }
        // Built-in roles are seeded idempotently: installs upgraded from older
        // versions gain roles added later (like `user` or `instance-manager`)
        // without touching customizations; only still-stock permission lists
        // are refreshed with new capabilities.
        seed_roles(&mut store.roles);
        // Warm the timing-attack dummy hash while the stores load.
        let dummy = tokio::task::spawn_blocking(|| hash_password("dummy-login-password"));
        let mut default_credentials = false;
        if store.users.is_empty() {
            let password = std::env::var("ADMIN_PASSWORD")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "123456".to_string());
            anyhow::ensure!(
                password.len() >= 6,
                "ADMIN_PASSWORD must have at least 6 characters"
            );
            let hash = tokio::task::spawn_blocking({
                let password = password.clone();
                move || hash_password(&password)
            })
            .await??;
            store.users.insert("Admin".into(), {
                let mut user = User {
                    username: "Admin".into(),
                    token_version: rand::random(),
                    password_hash: hash,
                    role: String::new(),
                    roles: Vec::new(),
                    display_name: "Administrator".into(),
                    created_at: timestamp(),
                    last_login: String::new(),
                    quotas: Default::default(),
                };
                user.set_roles(vec!["owner".into()]);
                user
            });
            default_credentials = std::env::var("ADMIN_PASSWORD")
                .ok()
                .filter(|s| !s.is_empty())
                .is_none();
            // Print once to the terminal, never record bootstrap credentials in application logs.
            eprintln!("Default account created\nUsername: Admin\nPassword: {password}");
            if default_credentials {
                eprintln!("WARNING: change this default password in Users after signing in.");
            }
        }
        let secret = config.secret_key.expose_secret();
        let key = if secret.is_empty() || secret == "change-me-in-production" {
            let path = config.data_dir.join("jwt_secret");
            match tokio::fs::read_to_string(&path).await {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let value = format!(
                        "{}{}",
                        uuid::Uuid::new_v4().simple(),
                        uuid::Uuid::new_v4().simple()
                    );
                    atomic_write(&path, value.as_bytes()).await?;
                    value
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            secret.to_owned()
        };
        anyhow::ensure!(
            key.len() >= 32,
            "SECRET_KEY must have at least 32 characters"
        );
        let dummy_hash = dummy.await??;
        let settings: serde_yaml::Value =
            match tokio::fs::read_to_string(config.data_dir.join(DEFAULT_PANEL_SETTINGS)).await {
                Ok(text) => serde_yaml::from_str(&text).context("invalid panel settings")?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_yaml::Value::Null,
                Err(e) => return Err(e.into()),
            };
        let login_enabled = settings
            .get("login_enabled")
            .and_then(serde_yaml::Value::as_bool)
            .unwrap_or(true);
        let curseforge_key = settings
            .get("curseforge_api_key")
            .and_then(serde_yaml::Value::as_str)
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_owned);
        let auth = Self {
            store: Arc::new(Mutex::new(store)),
            config,
            key: Arc::new(key.into()),
            work: Arc::new(Semaphore::new(4)),
            dummy_hash: Arc::new(dummy_hash),
            login_enabled: Arc::new(std::sync::atomic::AtomicBool::new(login_enabled)),
            default_credentials: Arc::new(std::sync::atomic::AtomicBool::new(default_credentials)),
            curseforge_key: Arc::new(std::sync::RwLock::new(curseforge_key)),
            throttle: Arc::new(super::throttle::LoginThrottle::default()),
        };
        {
            let store = auth.store.lock().await;
            auth.save_users(&store).await?;
            auth.save_roles(&store).await?;
        }
        Ok(auth)
    }
    pub(super) async fn save_users(&self, s: &Store) -> anyhow::Result<()> {
        atomic_write(
            &self.config.data_dir.join("users.yaml"),
            serde_yaml::to_string(&s.users)?.as_bytes(),
        )
        .await
    }
    pub(super) async fn save_roles(&self, s: &Store) -> anyhow::Result<()> {
        atomic_write(
            &self.config.data_dir.join("roles.yaml"),
            serde_yaml::to_string(&s.roles)?.as_bytes(),
        )
        .await
    }
}
