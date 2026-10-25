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
    "ROLES_MANAGE",
    "FILES_VIEW",
    "FILES_EDIT",
    "AUDIT_VIEW",
    "SETTINGS_VIEW",
    "SETTINGS_EDIT",
    "BROADCASTER_VIEW",
    "BROADCASTER_MANAGE",
    "INSTANCES_CREATE",
    "INSTANCES_VIEW_ALL",
    "INSTANCES_SHARE",
    "INSTANCES_DELETE",
    "INSTANCES_LIMITS",
];
/// Permissions that only make sense on one instance; they are the set instance
/// access roles may grant (global-only capabilities like USERS_* or
/// INSTANCES_CREATE are never implied by instance access).
pub const INSTANCE_PERMISSIONS: &[&str] = &[
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
    "FILES_VIEW",
    "FILES_EDIT",
    "BROADCASTER_VIEW",
    "INSTANCES_SHARE",
    "INSTANCES_DELETE",
    "INSTANCES_LIMITS",
];
#[derive(Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(default)]
    pub token_version: u64,
    pub username: String,
    pub password_hash: String,
    /// Display role (the first of `roles`); kept in sync by `set_roles`.
    #[serde(default)]
    pub role: String,
    /// All roles held by this user. Legacy single-role files (a `role:` field)
    /// migrate into this list on load.
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_login: String,
    /// Aggregate ceilings for this user's instances; None = unlimited.
    #[serde(default)]
    pub quotas: omnibedrock_server::instances::UserQuotas,
}
impl User {
    /// Canonical multi-role setter: keeps the legacy display field in sync.
    pub fn set_roles(&mut self, roles: Vec<String>) {
        self.roles = roles;
        self.role = self.roles.first().cloned().unwrap_or_default();
    }
    pub fn safe(&self) -> Value {
        json!({
            "username":self.username,
            "role":self.role,
            "roles":self.roles,
            "display_name":self.display_name,
            "created_at":self.created_at,
            "last_login":self.last_login,
            "quotas":self.quotas
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    /// Accent color as hex digits without '#', e.g. "5865F2".
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Hierarchy order (higher = more senior). 0 sorts last in listings.
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub builtin: bool,
    /// Sentinel held by the bootstrap Owner role. Never settable through the
    /// API — the platform requires at least one role carrying it and at least
    /// one user holding such a role.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub grants_owner: bool,
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
fn is_owner_in_store(store: &Store, user: &User) -> bool {
    user.roles
        .iter()
        .any(|name| store.roles.get(name).is_some_and(|role| role.grants_owner))
}
/// Union of the permissions granted by a set of role names. Unknown roles
/// contribute nothing (they may have been deleted after assignment).
fn union_permissions(store: &Store, roles: &[String]) -> Vec<String> {
    let mut perms: Vec<String> = Vec::new();
    for name in roles {
        if let Some(role) = store.roles.get(name) {
            for p in &role.permissions {
                if !perms.contains(p) {
                    perms.push(p.clone());
                }
            }
        }
    }
    perms
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
/// One stock role definition: name, display, color, description, position,
/// permissions, default flag.
struct StockRole {
    name: &'static str,
    display: &'static str,
    color: &'static str,
    description: &'static str,
    position: i64,
    permissions: Vec<&'static str>,
    default: bool,
}

fn stock_roles() -> Vec<StockRole> {
    vec![
        StockRole {
            name: "owner",
            display: "Owner",
            color: "F0B232",
            description: "Full control of the panel, every instance, users, roles and quotas.",
            position: 100,
            permissions: PERMISSIONS.to_vec(),
            default: false,
        },
        StockRole {
            name: "admin",
            display: "Admin",
            color: "5865F2",
            description: "Manages every instance and can delegate access, within owner-set quotas.",
            position: 80,
            permissions: PERMISSIONS.to_vec(),
            default: false,
        },
        StockRole {
            name: "moderator",
            display: "Moderator",
            color: "57F287",
            description: "Day-to-day operations: console, players, backups and file views.",
            position: 60,
            permissions: vec![
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
            default: false,
        },
        StockRole {
            name: "instance-manager",
            display: "Instance Manager",
            color: "EB459E",
            description: "Runtime control of instances it is granted on — the classic 'manage' share.",
            position: 50,
            permissions: vec![
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
                "FILES_VIEW",
                "FILES_EDIT",
                "BROADCASTER_VIEW",
            ],
            default: false,
        },
        StockRole {
            name: "user",
            display: "User",
            color: "3498DB",
            description: "Creates and runs their own instances within their quotas.",
            position: 40,
            permissions: vec![
                "CONSOLE_VIEW",
                "CONSOLE_SEND",
                "SERVER_VIEW",
                "SERVER_MANAGE",
                "INSTANCES_CREATE",
                "PLAYERS_VIEW",
                "PLAYERS_KICK",
                "PROPERTIES_VIEW",
                "PROPERTIES_EDIT",
                "ADDONS_VIEW",
                "ADDONS_MANAGE",
                "STORE_VIEW",
                "STORE_INSTALL",
                "BACKUPS_VIEW",
                "BACKUPS_CREATE",
                "BACKUPS_RESTORE",
                "FILES_VIEW",
                "FILES_EDIT",
            ],
            default: false,
        },
        StockRole {
            name: "viewer",
            display: "Viewer",
            color: "99AAB5",
            description: "Read-only: console, status and players. Default role for new users.",
            position: 20,
            permissions: vec!["CONSOLE_VIEW", "SERVER_VIEW", "PLAYERS_VIEW", "AUDIT_VIEW"],
            default: true,
        },
    ]
}

/// The permission lists stock roles shipped BEFORE the unified role system.
/// Used to refresh stock roles on upgrade: if an existing role still has the
/// exact old stock list, it is brought up to the new stock list; once an admin
/// has customized a role, its permissions are never touched again.
fn legacy_stock_permissions(name: &str) -> Option<Vec<&'static str>> {
    Some(match name {
        "owner" | "admin" => PERMISSIONS.to_vec(),
        "moderator" => vec![
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
        "user" => vec![
            "CONSOLE_VIEW",
            "CONSOLE_SEND",
            "SERVER_VIEW",
            "SERVER_MANAGE",
            "INSTANCES_CREATE",
            "PLAYERS_VIEW",
            "PLAYERS_KICK",
            "PROPERTIES_VIEW",
            "PROPERTIES_EDIT",
            "ADDONS_VIEW",
            "ADDONS_MANAGE",
            "STORE_VIEW",
            "STORE_INSTALL",
            "BACKUPS_VIEW",
            "BACKUPS_CREATE",
            "BACKUPS_RESTORE",
            "FILES_VIEW",
            "FILES_EDIT",
        ],
        "viewer" => vec!["CONSOLE_VIEW", "SERVER_VIEW", "PLAYERS_VIEW", "AUDIT_VIEW"],
        _ => return None,
    })
}

/// Seed/refresh the built-in roles. Insertion-only for customized roles:
/// existing entries are untouched unless their permissions still match the
/// exact pre-unification stock list, in which case they gain the new stock
/// permissions (ROLES_MANAGE, INSTANCES_*) and presentation defaults.
fn seed_roles(store: &mut BTreeMap<String, Role>) {
    for stock in stock_roles() {
        match store.entry(stock.name.to_string()) {
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(Role {
                    name: stock.name.into(),
                    display_name: stock.display.into(),
                    description: stock.description.into(),
                    color: stock.color.into(),
                    permissions: stock.permissions.iter().map(|s| s.to_string()).collect(),
                    position: stock.position,
                    is_default: stock.default,
                    builtin: true,
                    grants_owner: stock.name == "owner",
                    created_at: timestamp(),
                });
            }
            std::collections::btree_map::Entry::Occupied(mut e) => {
                let role = e.get_mut();
                if role.name == "owner" {
                    // The owner sentinel always stays on the owner role.
                    role.grants_owner = true;
                }
                let legacy = legacy_stock_permissions(stock.name);
                if let Some(legacy) = legacy
                    && role
                        .permissions
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        == legacy
                {
                    role.permissions = stock.permissions.iter().map(|s| s.to_string()).collect();
                }
                if role.color.is_empty() {
                    role.color = stock.color.into();
                }
                if role.description.is_empty() {
                    role.description = stock.description.into();
                }
                if role.position == 0 {
                    role.position = stock.position;
                }
                role.builtin = true;
            }
        }
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
    /// True when any of the user's roles carries the owner sentinel. This —
    /// not a role *name* — is what owner checks test, so the Owner role can be
    /// renamed without breaking the platform.
    pub async fn is_owner(&self, user: &User) -> bool {
        let store = self.store.lock().await;
        is_owner_in_store(&store, user)
    }
    /// Effective global permissions: the union of every role the user holds.
    /// Owner-sentinel holders implicitly hold everything. Permissions are
    /// purely additive — no role combination can revoke, and security-critical
    /// rules (quotas, owner-only operations) are enforced outside this list.
    pub async fn permissions(&self, user: &User) -> Vec<String> {
        let store = self.store.lock().await;
        if is_owner_in_store(&store, user) {
            return PERMISSIONS.iter().map(|s| s.to_string()).collect();
        }
        union_permissions(&store, &user.roles)
    }
    /// Permissions a user holds on ONE instance. Ownership of the record or
    /// the owner sentinel grants everything. Otherwise instance capability is
    /// deliberately independent of global role permissions: global perms
    /// govern the panel and the legacy default instance, while shared
    /// instances are governed by the roles the access entry assigns. Holders
    /// of the global INSTANCES_VIEW_ALL capability (panel admins) also project
    /// their global permissions onto every instance.
    pub async fn instance_permissions(
        &self,
        user: &User,
        record: &omnibedrock_server::instances::InstanceRecord,
    ) -> Vec<String> {
        if record.owner == user.username {
            return PERMISSIONS.iter().map(|s| s.to_string()).collect();
        }
        let store = self.store.lock().await;
        if is_owner_in_store(&store, user) {
            return PERMISSIONS.iter().map(|s| s.to_string()).collect();
        }
        let mut perms = Vec::new();
        let global = union_permissions(&store, &user.roles);
        if global.iter().any(|p| p == "INSTANCES_VIEW_ALL") {
            perms.extend(global);
        }
        if let Some(entry) = record.access.iter().find(|a| a.user == user.username) {
            for p in union_permissions(&store, entry.roles()) {
                if !perms.contains(&p) {
                    perms.push(p);
                }
            }
        }
        perms
    }
    pub async fn require(&self, user: &User, permission: &str) -> Result<()> {
        if !self.permissions(user).await.iter().any(|p| p == permission) {
            return Err(ApiError::forbidden(format!(
                "Missing permission: {permission}"
            )));
        }
        Ok(())
    }
    /// Instance-scoped permission gate. The migrated default instance keeps
    /// the legacy model (global role permissions only); other instances accept
    /// the union of global and access-entry role permissions.
    pub async fn require_instance(
        &self,
        user: &User,
        permission: &str,
        record: &omnibedrock_server::instances::InstanceRecord,
    ) -> Result<()> {
        if record.id == omnibedrock_server::instances::DEFAULT_INSTANCE {
            return self.require(user, permission).await;
        }
        if !self
            .instance_permissions(user, record)
            .await
            .iter()
            .any(|p| p == permission)
        {
            return Err(ApiError::forbidden(format!(
                "Missing permission: {permission}"
            )));
        }
        Ok(())
    }
    /// Whether the user may see a non-default instance at all: they own it,
    /// they hold the panel owner sentinel, they have an access entry on it, or
    /// they hold the global INSTANCES_VIEW_ALL capability.
    pub async fn can_access_instance(
        &self,
        user: &User,
        record: &omnibedrock_server::instances::InstanceRecord,
    ) -> bool {
        record.owner == user.username
            || record.access.iter().any(|a| a.user == user.username)
            || self.is_owner(user).await
            || self
                .permissions(user)
                .await
                .iter()
                .any(|p| p == "INSTANCES_VIEW_ALL")
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
        user.set_roles(current.roles.clone());
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
                .find(|u| is_owner_in_store(&store, u))
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
        let mut settings: serde_yaml::Value = match tokio::fs::read_to_string(
            self.config.data_dir.join(DEFAULT_PANEL_SETTINGS),
        )
        .await
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
    /// quotas for one user, if the account exists.
    pub async fn user_quotas(
        &self,
        username: &str,
    ) -> Option<omnibedrock_server::instances::UserQuotas> {
        self.store
            .lock()
            .await
            .users
            .get(username)
            .map(|u| u.quotas.clone())
    }
    /// The bootstrap owner for the default instance's record.
    pub async fn first_owner_username(&self) -> String {
        let store = self.store.lock().await;
        store
            .users
            .values()
            .find(|u| is_owner_in_store(&store, u))
            .map(|u| u.username.clone())
            .unwrap_or_else(|| "Admin".into())
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
        let store = self.store.lock().await;
        let mut list: Vec<Value> = store
            .roles
            .values()
            .map(|r| {
                let mut v = json!(r);
                v["user_count"] = json!(
                    store
                        .users
                        .values()
                        .filter(|u| u.roles.iter().any(|name| name == &r.name))
                        .count()
                );
                v
            })
            .collect();
        list.sort_by(|a, b| {
            b["position"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&a["position"].as_i64().unwrap_or(0))
                .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
        });
        json!(list)
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
        // Legacy single-role bodies ({"role": "x"}) normalize to a one-role list.
        let body = body.map(|mut b| {
            if b.get("roles").is_none() && b["role"].is_string() {
                let role = b["role"].as_str().unwrap_or_default().to_string();
                b["roles"] = json!([role]);
            }
            b
        });
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
        let actor_is_owner = is_owner_in_store(&next, actor);
        if let Some(body) = body {
            if create && next.users.contains_key(name) {
                return Err(ApiError::conflict("Username already exists"));
            }
            let mut user = if create {
                let mut u = User {
                    username: name.into(),
                    token_version: rand::random(),
                    password_hash: hash
                        .clone()
                        .ok_or_else(|| ApiError::bad("Password required"))?,
                    role: String::new(),
                    roles: Vec::new(),
                    display_name: name.into(),
                    created_at: timestamp(),
                    last_login: String::new(),
                    quotas: Default::default(),
                };
                u.set_roles(vec![
                    next.roles
                        .values()
                        .find(|r| r.is_default)
                        .map_or("viewer".into(), |r| r.name.clone()),
                ]);
                u
            } else {
                next.users
                    .get(name)
                    .cloned()
                    .ok_or_else(|| ApiError::missing("User not found"))?
            };
            let target_is_owner = is_owner_in_store(&next, &user);
            if let Some(roles_value) = body.get("roles") {
                let mut roles: Vec<String> = match roles_value {
                    Value::Array(arr) => arr
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(String::from)
                                .ok_or_else(|| ApiError::bad("Invalid role list"))
                        })
                        .collect::<Result<_>>()?,
                    // Legacy single-role body: {"role": "moderator"}
                    Value::String(s) => vec![s.clone()],
                    _ => return Err(ApiError::bad("Invalid role list")),
                };
                if roles.is_empty() {
                    return Err(ApiError::bad("At least one role is required"));
                }
                roles.sort();
                roles.dedup();
                for r in &roles {
                    if !next.roles.contains_key(r) {
                        return Err(ApiError::bad(format!("Unknown role: {r}")));
                    }
                }
                let grants_owner = roles
                    .iter()
                    .any(|r| next.roles.get(r).is_some_and(|role| role.grants_owner));
                if !actor_is_owner && (target_is_owner || grants_owner) {
                    return Err(ApiError::forbidden(
                        "Only owners may change owner accounts or assign the Owner role",
                    ));
                }
                if !actor_is_owner {
                    // Delegation guard: an admin can only hand out permissions
                    // they hold themselves, so no chain of edits escalates.
                    let granted = union_permissions(&next, &roles);
                    let held = union_permissions(&next, &actor.roles);
                    if let Some(missing) = granted.iter().find(|p| !held.contains(p)) {
                        return Err(ApiError::forbidden(format!(
                            "You cannot grant the role '{role}' — it includes {perm}, which you do not hold",
                            role = roles.join(", "),
                            perm = missing
                        )));
                    }
                }
                user.set_roles(roles);
            }
            if !actor_is_owner && (target_is_owner || is_owner_in_store(&next, &user)) {
                return Err(ApiError::forbidden("Only owners may change owner accounts"));
            }
            if let Some(name) = body["display_name"].as_str() {
                user.display_name = name.into();
            }
            if body.get("quotas").is_some_and(|v| !v.is_null()) {
                if !actor_is_owner {
                    return Err(ApiError::forbidden("Only owners may change quotas"));
                }
                let quotas: omnibedrock_server::instances::UserQuotas =
                    serde_json::from_value(body["quotas"].clone())
                        .map_err(|e| ApiError::bad(format!("Invalid quotas: {e}")))?;
                quotas
                    .validate()
                    .map_err(|e| ApiError::bad(e.to_string()))?;
                user.quotas = quotas;
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
            if !next.users.values().any(|u| is_owner_in_store(&next, u)) {
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
            if !actor_is_owner && is_owner_in_store(&next, &user) {
                return Err(ApiError::forbidden("Only owners may delete owner accounts"));
            }
            if !next.users.values().any(|u| is_owner_in_store(&next, u)) {
                return Err(ApiError::bad("At least one owner is required"));
            }
            self.save_users(&next).await?;
            *store = next;
            Ok(json!({"success":true,"message":format!("Deleted user {name}")}))
        }
    }
    /// Create, edit or delete a role. Returns the stored role plus, when the
    /// role's key was renamed, the old key so callers can rewrite references
    /// (instance access entries live in a different store).
    pub async fn mutate_role(
        &self,
        actor: &User,
        name: &str,
        body: Option<Value>,
        create: bool,
    ) -> Result<(Value, Option<String>)> {
        omnibedrock_core::files::component(name)?;
        let mut store = self.store.lock().await;
        let mut next = store.clone();
        let actor_is_owner = is_owner_in_store(&next, actor);
        let mut renamed_from: Option<String> = None;
        if let Some(body) = body {
            // Duplicate: seed the new role from an existing one before edits.
            let source = if create {
                body["duplicate_of"]
                    .as_str()
                    .and_then(|src| next.roles.get(src).cloned())
            } else {
                None
            };
            if create && next.roles.contains_key(name) {
                return Err(ApiError::conflict("Role already exists"));
            }
            let mut role = if create {
                Role {
                    name: name.into(),
                    display_name: body["display_name"].as_str().unwrap_or(name).into(),
                    description: String::new(),
                    color: String::new(),
                    permissions: Vec::new(),
                    position: next.roles.values().map(|r| r.position).min().unwrap_or(0) - 1,
                    is_default: false,
                    builtin: false,
                    grants_owner: false,
                    created_at: timestamp(),
                }
            } else {
                next.roles
                    .get(name)
                    .cloned()
                    .ok_or_else(|| ApiError::missing("Role not found"))?
            };
            if let Some(src) = source {
                role.permissions = src.permissions.clone();
                role.description = src.description.clone();
                role.color = src.color.clone();
                role.position = src.position;
            }
            if let Some(perms) = body["permissions"].as_array() {
                let requested: Vec<String> = perms
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(String::from)
                            .ok_or_else(|| ApiError::bad("Invalid permission"))
                    })
                    .collect::<Result<_>>()?;
                if requested.iter().any(|p| !PERMISSIONS.contains(&p.as_str())) {
                    return Err(ApiError::bad("Unknown permission"));
                }
                // Delegation guard: without the owner sentinel you cannot
                // author a role granting permissions you do not hold yourself.
                if !actor_is_owner {
                    let held = union_permissions(&next, &actor.roles);
                    if let Some(missing) = requested.iter().find(|p| !held.contains(p)) {
                        return Err(ApiError::forbidden(format!(
                            "You cannot grant {perm} — you do not hold it yourself",
                            perm = missing
                        )));
                    }
                }
                role.permissions = requested;
            }
            if let Some(display) = body["display_name"].as_str() {
                role.display_name = display.into();
            }
            if let Some(desc) = body["description"].as_str() {
                role.description = desc.into();
            }
            if let Some(color) = body["color"].as_str() {
                let color = color.trim().trim_start_matches('#').to_ascii_uppercase();
                if !color.is_empty()
                    && (color.len() != 6 || !color.chars().all(|c| c.is_ascii_hexdigit()))
                {
                    return Err(ApiError::bad(
                        "Color must be a 6-digit hex value like 5865F2",
                    ));
                }
                role.color = color;
            }
            if let Some(position) = body["position"].as_i64() {
                role.position = position;
            }
            if let Some(default) = body["is_default"].as_bool() {
                role.is_default = default;
            }
            if role.is_default {
                for r in next.roles.values_mut() {
                    r.is_default = false;
                }
            }
            // Rename (custom roles only; built-ins keep their stable keys).
            if let Some(new_name) = body["name"].as_str()
                && new_name != name
            {
                if role.builtin {
                    return Err(ApiError::bad(
                        "Built-in roles keep their key; change the display name instead",
                    ));
                }
                omnibedrock_core::files::component(new_name)?;
                if next.roles.contains_key(new_name) {
                    return Err(ApiError::conflict("Role already exists"));
                }
                for user in next.users.values_mut() {
                    for r in user.roles.iter_mut() {
                        if r == name {
                            *r = new_name.into();
                        }
                    }
                    user.set_roles(user.roles.clone());
                }
                role.name = new_name.into();
                renamed_from = Some(name.to_string());
            }
            // The owner sentinel is never API-settable and never removable.
            role.grants_owner = next.roles.get(name).is_some_and(|r| r.grants_owner) && !create;
            next.roles.insert(role.name.clone(), role.clone());
            if renamed_from.is_some() {
                // Renames keep the old key from shadowing the new one.
                next.roles.remove(name);
            }
            self.save_roles(&next).await?;
            *store = next;
            Ok((json!(role), renamed_from))
        } else {
            let role = next
                .roles
                .get(name)
                .cloned()
                .ok_or_else(|| ApiError::missing("Role not found"))?;
            if role.grants_owner {
                return Err(ApiError::bad("The Owner role cannot be deleted"));
            }
            if role.builtin {
                return Err(ApiError::bad(
                    "Built-in roles cannot be deleted; edit them instead",
                ));
            }
            if next
                .users
                .values()
                .any(|u| u.roles.iter().any(|r| r == name))
            {
                return Err(ApiError::conflict("Role is assigned to users"));
            }
            next.roles
                .remove(name)
                .ok_or_else(|| ApiError::missing("Role not found"))?;
            self.save_roles(&next).await?;
            *store = next;
            Ok((
                json!({"success":true,"message":format!("Role {name} deleted")}),
                None,
            ))
        }
    }
}
