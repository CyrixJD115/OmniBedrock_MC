use anyhow::Context as _;
use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString, rand_core::OsRng},
};
use axum::http::StatusCode;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use omnibedrock_core::{
    config::Config,
    error::{ApiError, Result},
    files::atomic_write,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, Semaphore};

pub const PERMISSIONS: &[&str] = &[
    "CONSOLE_VIEW",
    "CONSOLE_SEND",
    "SERVER_VIEW",
    "SERVER_MANAGE",
    "PLAYERS_VIEW",
    "PLAYERS_KICK",
    "PLAYERS_BAN",
    "PLAYERS_OP",
    "PROPERTIES_VIEW",
    "PROPERTIES_EDIT",
    "ADDONS_VIEW",
    "ADDONS_MANAGE",
    "STORE_VIEW",
    "STORE_INSTALL",
    "BACKUPS_VIEW",
    "BACKUPS_CREATE",
    "BACKUPS_RESTORE",
    "BACKUPS_DELETE",
    "USERS_VIEW",
    "USERS_CREATE",
    "USERS_EDIT",
    "USERS_DELETE",
    "FILES_VIEW",
    "FILES_EDIT",
    "AUDIT_VIEW",
    "SETTINGS_VIEW",
    "SETTINGS_EDIT",
    "BROADCASTER_VIEW",
    "BROADCASTER_MANAGE",
];
#[derive(Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(default)]
    pub token_version: u64,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_login: String,
}
impl User {
    pub fn safe(&self) -> Value {
        json!({"username":self.username,"role":self.role,"display_name":self.display_name,"created_at":self.created_at,"last_login":self.last_login})
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub created_at: String,
}
#[derive(Default, Clone)]
struct Store {
    users: BTreeMap<String, User>,
    roles: BTreeMap<String, Role>,
}
#[derive(Clone)]
pub struct Auth {
    store: Arc<Mutex<Store>>,
    config: Arc<Config>,
    key: Arc<SecretString>,
    work: Arc<Semaphore>,
    dummy_hash: Arc<String>,
    /// When false, every request is served as the owner account without a token.
    login_enabled: Arc<std::sync::atomic::AtomicBool>,
    /// True until the seeded default password is changed.
    default_credentials: Arc<std::sync::atomic::AtomicBool>,
    /// CurseForge API key saved from the Settings page; overrides the .env value.
    curseforge_key: Arc<std::sync::RwLock<Option<String>>>,
}
const DEFAULT_PANEL_SETTINGS: &str = "panel_settings.yaml";
#[derive(Serialize, Deserialize)]
struct Claims {
    #[serde(default)]
    ver: u64,
    sub: String,
    role: String,
    iat: u64,
    exp: u64,
}
fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn hash_password(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("password hash: {e}"))
}
fn verify_password(password: &str, stored: &str) -> bool {
    if stored.starts_with("$argon2") {
        return PasswordHash::new(stored).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        });
    }
    // Existing Python PBKDF2-HMAC-SHA256 accounts upgrade to Argon2id at login.
    if let Some((salt, key)) = stored.split_once(':')
        && let (Ok(salt), Ok(key)) = (hex::decode(salt), hex::decode(key))
    {
        let expected = pbkdf2::pbkdf2_hmac_array::<Sha256, 32>(password.as_bytes(), &salt, 100_000);
        return bool::from(expected.as_slice().ct_eq(&key));
    }
    false
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
        if store.roles.is_empty() {
            for (name, display, permissions, default) in [
                ("owner", "Owner", PERMISSIONS.to_vec(), false),
                ("admin", "Admin", PERMISSIONS.to_vec(), false),
                (
                    "moderator",
                    "Moderator",
                    vec![
                        "CONSOLE_VIEW",
                        "CONSOLE_SEND",
                        "SERVER_VIEW",
                        "PLAYERS_VIEW",
                        "PLAYERS_KICK",
                        "PROPERTIES_VIEW",
                        "ADDONS_VIEW",
                        "BACKUPS_VIEW",
                        "BACKUPS_CREATE",
                        "BACKUPS_RESTORE",
                        "BACKUPS_DELETE",
                        "FILES_VIEW",
                        "AUDIT_VIEW",
                        "SETTINGS_VIEW",
                        "BROADCASTER_VIEW",
                    ],
                    false,
                ),
                (
                    "viewer",
                    "Viewer",
                    vec!["CONSOLE_VIEW", "SERVER_VIEW", "PLAYERS_VIEW", "AUDIT_VIEW"],
                    true,
                ),
            ] {
                store.roles.insert(
                    name.into(),
                    Role {
                        name: name.into(),
                        display_name: display.into(),
                        permissions: permissions.into_iter().map(String::from).collect(),
                        is_default: default,
                        created_at: timestamp(),
                    },
                );
            }
        }
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
            store.users.insert(
                "Admin".into(),
                User {
                    username: "Admin".into(),
                    token_version: rand::random(),
                    password_hash: hash,
                    role: "owner".into(),
                    display_name: "Administrator".into(),
                    created_at: timestamp(),
                    last_login: String::new(),
                },
            );
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
        };
        {
            let store = auth.store.lock().await;
            auth.save_users(&store).await?;
            auth.save_roles(&store).await?;
        }
        Ok(auth)
    }
    async fn save_users(&self, s: &Store) -> anyhow::Result<()> {
        atomic_write(
            &self.config.data_dir.join("users.yaml"),
            serde_yaml::to_string(&s.users)?.as_bytes(),
        )
        .await
    }
    async fn save_roles(&self, s: &Store) -> anyhow::Result<()> {
        atomic_write(
            &self.config.data_dir.join("roles.yaml"),
            serde_yaml::to_string(&s.roles)?.as_bytes(),
        )
        .await
    }
    pub async fn permissions(&self, user: &User) -> Vec<String> {
        if user.role == "owner" {
            return PERMISSIONS.iter().map(|s| s.to_string()).collect();
        }
        self.store
            .lock()
            .await
            .roles
            .get(&user.role)
            .map_or_else(Vec::new, |r| r.permissions.clone())
    }
    pub async fn require(&self, user: &User, permission: &str) -> Result<()> {
        if !self.permissions(user).await.iter().any(|p| p == permission) {
            return Err(ApiError::forbidden(format!(
                "Missing permission: {permission}"
            )));
        }
        Ok(())
    }
    pub async fn me(&self, user: &User) -> Value {
        let mut v = user.safe();
        v["permissions"] = json!(self.permissions(user).await);
        v
    }
    pub async fn authenticate(&self, username: String, password: String) -> Result<Value> {
        if password.len() > 1024 || username.len() > 128 {
            return Err(ApiError::bad("Credentials too long"));
        }
        let permit =
            self.work.clone().try_acquire_owned().map_err(|_| {
                ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Too many login attempts")
            })?;
        let user = self.store.lock().await.users.get(&username).cloned();
        let stored = user.as_ref().map_or_else(
            || self.dummy_hash.as_ref().clone(),
            |u| u.password_hash.clone(),
        );
        let old_hash = stored.clone();
        let (valid, upgrade) = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let valid = verify_password(&password, &stored);
            let upgrade = if valid && !stored.starts_with("$argon2") {
                Some(hash_password(&password))
            } else {
                None
            };
            (valid, upgrade)
        })
        .await
        .map_err(anyhow::Error::from)?;
        let mut user = user.filter(|_| valid).ok_or_else(|| {
            ApiError::new(StatusCode::UNAUTHORIZED, "Invalid username or password")
        })?;
        let mut store = self.store.lock().await;
        let current = store
            .users
            .get(&username)
            .filter(|u| u.password_hash == old_hash)
            .ok_or_else(|| {
                ApiError::new(StatusCode::UNAUTHORIZED, "Account changed; log in again")
            })?;
        user.role = current.role.clone();
        if let Some(hash) = upgrade {
            user.password_hash = hash?;
        }
        user.last_login = timestamp();
        let mut next = store.clone();
        next.users.insert(username, user.clone());
        self.save_users(&next).await?;
        *store = next;
        drop(store);
        let now = chrono::Utc::now().timestamp() as u64;
        let claims = Claims {
            ver: user.token_version,
            sub: user.username.clone(),
            role: user.role.clone(),
            iat: now,
            exp: now + 86400 * 7,
        };
        let key = Sha256::digest(self.key.expose_secret().as_bytes());
        let token = jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&key),
        )
        .map_err(anyhow::Error::from)?;
        Ok(json!({"token":token,"user":self.me(&user).await}))
    }
    pub async fn verify(&self, token: &str) -> Result<User> {
        let key = Sha256::digest(self.key.expose_secret().as_bytes());
        let mut validation = Validation::new(Algorithm::HS256);
        validation.leeway = 0;
        let claims =
            jsonwebtoken::decode::<Claims>(token, &DecodingKey::from_secret(&key), &validation)
                .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid or expired token"))?
                .claims;
        self.store
            .lock()
            .await
            .users
            .get(&claims.sub)
            .filter(|u| u.token_version == claims.ver)
            .cloned()
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "User not found"))
    }
    /// Resolve the acting user. With login disabled every request is served as
    /// the owner account, token or not; with it enabled a valid token is required.
    pub async fn resolve_user(&self, token: Option<&str>) -> Result<User> {
        if !self.login_enabled() {
            let store = self.store.lock().await;
            return store
                .users
                .values()
                .find(|u| u.role == "owner")
                .or_else(|| store.users.values().next())
                .cloned()
                .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "No accounts exist"));
        }
        let token = token
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Not authenticated"))?;
        self.verify(token).await
    }
    pub fn login_enabled(&self) -> bool {
        self.login_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    pub async fn set_login_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        self.login_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
        self.update_panel_settings(|settings| {
            settings["login_enabled"] = enabled.into();
        })
        .await
    }
    /// The API key saved via Settings, if any. Empty string clears the override.
    pub async fn curseforge_key_override(&self) -> Option<String> {
        self.curseforge_key.read().unwrap().clone()
    }
    /// Hint for the UI: last 4 characters of the stored override, never the key.
    pub async fn curseforge_key_hint(&self) -> Option<String> {
        self.curseforge_key.read().unwrap().as_ref().map(|key| {
            let tail: String = key
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("…{tail}")
        })
    }
    pub async fn set_curseforge_key(&self, key: &str) -> anyhow::Result<()> {
        let trimmed = key.trim();
        let stored = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        *self.curseforge_key.write().unwrap() = stored.clone();
        self.update_panel_settings(|settings| {
            settings["curseforge_api_key"] = stored
                .clone()
                .map_or(serde_yaml::Value::Null, serde_yaml::Value::String);
        })
        .await
    }
    /// Merge one or more keys into `panel_settings.yaml`, preserving the rest.
    async fn update_panel_settings(
        &self,
        mutate: impl FnOnce(&mut serde_yaml::Value),
    ) -> anyhow::Result<()> {
        let mut settings: serde_yaml::Value =
            match tokio::fs::read_to_string(self.config.data_dir.join(DEFAULT_PANEL_SETTINGS)).await
            {
                Ok(text) => serde_yaml::from_str(&text).unwrap_or_else(|_| serde_yaml::Value::Null),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_yaml::Value::Null,
                Err(e) => return Err(e.into()),
            };
        if !settings.is_mapping() {
            settings = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        mutate(&mut settings);
        atomic_write(
            &self.config.data_dir.join(DEFAULT_PANEL_SETTINGS),
            serde_yaml::to_string(&settings)?.as_bytes(),
        )
        .await
    }
    /// Public login-page info: whether the login form is used, and whether the
    /// seeded default password is still in place.
    pub fn panel_info(&self) -> Value {
        json!({
            "login_enabled": self.login_enabled(),
            "default_credentials": self.default_credentials.load(std::sync::atomic::Ordering::Relaxed),
        })
    }
    pub async fn users(&self) -> Value {
        json!(
            self.store
                .lock()
                .await
                .users
                .values()
                .map(User::safe)
                .collect::<Vec<_>>()
        )
    }
    pub async fn roles(&self) -> Value {
        json!(self.store.lock().await.roles.values().collect::<Vec<_>>())
    }
    pub async fn role(&self, name: &str) -> Result<Value> {
        self.store
            .lock()
            .await
            .roles
            .get(name)
            .map(|r| json!(r))
            .ok_or_else(|| ApiError::missing("Role not found"))
    }
    pub async fn mutate_user(
        &self,
        actor: &User,
        name: &str,
        body: Option<Value>,
        create: bool,
    ) -> Result<Value> {
        omnibedrock_core::files::component(name)?;
        let hash = if let Some(password) = body.as_ref().and_then(|b| b["password"].as_str()) {
            if password.len() < 6 || password.len() > 1024 {
                return Err(ApiError::bad("Password must have 6..1024 characters"));
            }
            let permit = self.work.clone().try_acquire_owned().map_err(|_| {
                ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Password service busy")
            })?;
            let password = password.to_owned();
            Some(
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    hash_password(&password)
                })
                .await
                .map_err(anyhow::Error::from)??,
            )
        } else {
            None
        };
        let mut store = self.store.lock().await;
        let mut next = store.clone();
        if let Some(body) = body {
            if create && next.users.contains_key(name) {
                return Err(ApiError::conflict("Username already exists"));
            }
            let mut user = if create {
                User {
                    username: name.into(),
                    token_version: rand::random(),
                    password_hash: hash
                        .clone()
                        .ok_or_else(|| ApiError::bad("Password required"))?,
                    role: next
                        .roles
                        .values()
                        .find(|r| r.is_default)
                        .map_or("viewer".into(), |r| r.name.clone()),
                    display_name: name.into(),
                    created_at: timestamp(),
                    last_login: String::new(),
                }
            } else {
                next.users
                    .get(name)
                    .cloned()
                    .ok_or_else(|| ApiError::missing("User not found"))?
            };
            if let Some(role) = body["role"].as_str() {
                if !next.roles.contains_key(role) {
                    return Err(ApiError::bad("Unknown role"));
                }
                if actor.role != "owner" && (role == "owner" || user.role == "owner") {
                    return Err(ApiError::forbidden("Only owners may change owner accounts"));
                }
                user.role = role.into();
            }
            if actor.role != "owner" && user.role == "owner" {
                return Err(ApiError::forbidden("Only owners may change owner accounts"));
            }
            if let Some(name) = body["display_name"].as_str() {
                user.display_name = name.into();
            }
            if let Some(hash) = hash {
                user.token_version = rand::random();
                user.password_hash = hash;
                if name == "Admin" {
                    // The seeded default password no longer applies.
                    self.default_credentials
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
            next.users.insert(name.into(), user.clone());
            if !next.users.values().any(|u| u.role == "owner") {
                return Err(ApiError::bad("At least one owner is required"));
            }
            self.save_users(&next).await?;
            *store = next;
            Ok(user.safe())
        } else {
            if actor.username == name {
                return Err(ApiError::bad("Cannot delete yourself"));
            }
            let user = next
                .users
                .remove(name)
                .ok_or_else(|| ApiError::missing("User not found"))?;
            if actor.role != "owner" && user.role == "owner" {
                return Err(ApiError::forbidden("Only owners may delete owner accounts"));
            }
            if !next.users.values().any(|u| u.role == "owner") {
                return Err(ApiError::bad("At least one owner is required"));
            }
            self.save_users(&next).await?;
            *store = next;
            Ok(json!({"success":true,"message":format!("Deleted user {name}")}))
        }
    }
    pub async fn mutate_role(
        &self,
        name: &str,
        body: Option<Value>,
        create: bool,
    ) -> Result<Value> {
        omnibedrock_core::files::component(name)?;
        let mut store = self.store.lock().await;
        let mut next = store.clone();
        if let Some(body) = body {
            if create && next.roles.contains_key(name) {
                return Err(ApiError::conflict("Role already exists"));
            }
            let mut role = if create {
                Role {
                    name: name.into(),
                    display_name: name.into(),
                    permissions: vec![],
                    is_default: false,
                    created_at: timestamp(),
                }
            } else {
                next.roles
                    .get(name)
                    .cloned()
                    .ok_or_else(|| ApiError::missing("Role not found"))?
            };
            if let Some(perms) = body["permissions"].as_array() {
                role.permissions = perms
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(String::from)
                            .ok_or_else(|| ApiError::bad("Invalid permission"))
                    })
                    .collect::<Result<_>>()?;
                if role
                    .permissions
                    .iter()
                    .any(|p| !PERMISSIONS.contains(&p.as_str()))
                {
                    return Err(ApiError::bad("Unknown permission"));
                }
            }
            if let Some(display) = body["display_name"].as_str() {
                role.display_name = display.into();
            }
            if let Some(default) = body["is_default"].as_bool() {
                role.is_default = default;
            }
            if role.is_default {
                for r in next.roles.values_mut() {
                    r.is_default = false;
                }
            }
            next.roles.insert(name.into(), role.clone());
            self.save_roles(&next).await?;
            *store = next;
            Ok(json!(role))
        } else {
            if name == "owner" {
                return Err(ApiError::bad("Cannot delete owner role"));
            }
            if next.users.values().any(|u| u.role == name) {
                return Err(ApiError::conflict("Role is assigned to users"));
            }
            next.roles
                .remove(name)
                .ok_or_else(|| ApiError::missing("Role not found"))?;
            self.save_roles(&next).await?;
            *store = next;
            Ok(json!({"success":true,"message":format!("Role {name} deleted")}))
        }
    }
}
