//! Core auth data types — the user and role records and the shared in-memory
//! store — plus the small helpers (`timestamp`, owner detection and permission
//! union lookups over the store) used across the auth submodules.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
pub(super) struct Store {
    pub(super) users: BTreeMap<String, User>,
    pub(super) roles: BTreeMap<String, Role>,
}

pub(super) fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub(super) fn is_owner_in_store(store: &Store, user: &User) -> bool {
    user.roles
        .iter()
        .any(|name| store.roles.get(name).is_some_and(|role| role.grants_owner))
}
/// Union of the permissions granted by a set of role names. Unknown roles
/// contribute nothing (they may have been deleted after assignment).
pub(super) fn union_permissions(store: &Store, roles: &[String]) -> Vec<String> {
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
