//! Personal API keys: long-lived credentials for scripts and integrations
//! that ride the panel's existing URL and port — no extra listener.
//!
//! A key belongs to exactly one user and acts with that user's permissions
//! (never more). Requests present it as
//!
//! ```text
//! X-API-User: <username>
//! X-API-Key:  omb_<id>_<secret>
//! ```
//!
//! or, for tools that only speak bearer auth, `Authorization: Bearer
//! omb_<id>_<secret>` (the username is then taken from the key record).
//!
//! Only a SHA-256 of the full key is stored (`data/api_keys.yaml`); the
//! plaintext is shown once at creation. Keys can be read-only (GET only),
//! carry an optional expiry, and are revoked on demand. A request
//! authenticated by a key can never mint, list, or revoke keys, so a leaked
//! key cannot entrench itself. Deleting the owning user invalidates its keys.

use axum::http::StatusCode;
use omnibedrock_core::{
    error::{ApiError, Result},
    files::atomic_write,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::Auth;
use super::types::{User, timestamp};

pub(super) const API_KEYS_FILE: &str = "api_keys.yaml";
/// Prefix every panel key carries, so secret scanners and humans can spot one.
const KEY_PREFIX: &str = "omb_";
/// Per-user ceiling; keys are cheap to rotate, an unbounded list is not.
const MAX_KEYS_PER_USER: usize = 25;
/// `last_used` is persisted at most this often per key (it is also kept in
/// memory), so a busy integration does not rewrite the file every request.
const LAST_USED_PERSIST_SECS: i64 = 60;

/// What a key may do on top of its owner's permissions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyScope {
    /// Everything the owning user may do.
    #[default]
    Full,
    /// GET requests only (status, console tail, listings, downloads).
    ReadOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ApiKey {
    /// Public identifier embedded in the key (`omb_<id>_…`); safe to show.
    pub(super) id: String,
    pub(super) user: String,
    pub(super) name: String,
    /// Hex SHA-256 of the full plaintext key.
    pub(super) hash: String,
    #[serde(default)]
    pub(super) scope: KeyScope,
    pub(super) created_at: String,
    #[serde(default)]
    pub(super) last_used: String,
    #[serde(default)]
    pub(super) last_ip: String,
    /// RFC 3339; empty means the key never expires.
    #[serde(default)]
    pub(super) expires_at: String,
}

impl ApiKey {
    fn public(&self) -> Value {
        json!({
            "id": self.id,
            "user": self.user,
            "name": self.name,
            "scope": self.scope,
            "prefix": format!("{KEY_PREFIX}{}_", self.id),
            "created_at": self.created_at,
            "last_used": self.last_used,
            "last_ip": self.last_ip,
            "expires_at": self.expires_at,
            "expired": self.expired(),
        })
    }
    fn expired(&self) -> bool {
        !self.expires_at.is_empty()
            && chrono::DateTime::parse_from_rfc3339(&self.expires_at)
                .is_ok_and(|at| at < chrono::Utc::now())
    }
}

fn digest(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// Split `omb_<id>_<secret>` into its id; anything else is not a panel key.
fn key_id(key: &str) -> Option<&str> {
    let rest = key.strip_prefix(KEY_PREFIX)?;
    let (id, secret) = rest.split_once('_')?;
    (id.len() == 12
        && id.chars().all(|c| c.is_ascii_alphanumeric())
        && secret.len() >= 32
        && secret.chars().all(|c| c.is_ascii_alphanumeric()))
    .then_some(id)
}

/// True when a bearer value is shaped like a panel API key rather than a JWT.
pub fn looks_like_api_key(value: &str) -> bool {
    key_id(value.trim()).is_some()
}

fn random_alnum(len: usize) -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rngs::OsRng;
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

pub(super) async fn load_keys(path: &std::path::Path) -> anyhow::Result<Vec<ApiKey>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) if text.trim().is_empty() => Ok(Vec::new()),
        Ok(text) => Ok(serde_yaml::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

impl Auth {
    async fn save_keys(&self, keys: &[ApiKey]) -> anyhow::Result<()> {
        atomic_write(
            &self.config.data_dir.join(API_KEYS_FILE),
            serde_yaml::to_string(keys)?.as_bytes(),
        )
        .await
    }

    /// Resolve the acting user from an API key. `claimed_user` is the
    /// `X-API-User` header when present; it must name the key's owner.
    /// Failures are throttled per source like logins.
    pub async fn resolve_api_key(
        &self,
        claimed_user: Option<&str>,
        key: &str,
        ip: &str,
    ) -> Result<User> {
        let unauthorized = || ApiError::new(StatusCode::UNAUTHORIZED, "Invalid API key");
        let source_key = format!("apikey\0{ip}");
        // Per-source only: a shared counter would let anyone lock every
        // integration out by spraying bad keys.
        let throttle_keys = [source_key.as_str()];
        if let Err(wait) = self.throttle.check(&throttle_keys) {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                format!(
                    "Too many invalid API keys; try again in {}s",
                    wait.as_secs().max(1)
                ),
            ));
        }
        let key = key.trim();
        let fail = |this: &Self| {
            this.throttle.record_failure(&throttle_keys);
            unauthorized()
        };
        let Some(id) = key_id(key) else {
            return Err(fail(self));
        };
        let presented = digest(key);
        let mut keys = self.api_keys.lock().await;
        let Some(index) = keys.iter().position(|k| k.id == id) else {
            drop(keys);
            return Err(fail(self));
        };
        let record = &keys[index];
        let hash_ok: bool = record.hash.as_bytes().ct_eq(presented.as_bytes()).into();
        let user_ok = claimed_user.is_none_or(|u| u.trim() == record.user);
        if !hash_ok || !user_ok {
            drop(keys);
            return Err(fail(self));
        }
        if record.expired() {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "API key expired"));
        }
        let scope = record.scope;
        let owner = record.user.clone();
        let user = self
            .store
            .lock()
            .await
            .users
            .get(&owner)
            .cloned()
            .ok_or_else(unauthorized)?;
        // Usage stamp: always in memory, persisted at a bounded rate.
        let now = chrono::Utc::now();
        let persist =
            chrono::DateTime::parse_from_rfc3339(&keys[index].last_used).map_or(true, |t| {
                (now - t.with_timezone(&chrono::Utc)).num_seconds() >= LAST_USED_PERSIST_SECS
            }) || keys[index].last_ip != ip;
        keys[index].last_used = now.to_rfc3339();
        keys[index].last_ip = ip.to_owned();
        if persist {
            let snapshot = keys.clone();
            drop(keys);
            if let Err(e) = self.save_keys(&snapshot).await {
                tracing::warn!(%e, "api key usage stamp not persisted");
            }
        }
        let mut user = user;
        user.key_scope = Some(scope);
        Ok(user)
    }

    /// Keys visible to `actor`: their own, or every key when `all` and the
    /// actor may manage users.
    pub async fn list_api_keys(&self, actor: &User, all: bool) -> Result<Value> {
        reject_key_session(actor)?;
        let everyone = all && self.can_manage_all_keys(actor).await;
        let keys = self.api_keys.lock().await;
        let items: Vec<Value> = keys
            .iter()
            .filter(|k| everyone || k.user == actor.username)
            .map(ApiKey::public)
            .collect();
        Ok(json!(items))
    }

    /// Create a key for `actor` (or, for user managers, for `for_user`).
    /// Returns the record plus the plaintext key, which is never stored.
    pub async fn create_api_key(&self, actor: &User, body: &Value) -> Result<Value> {
        reject_key_session(actor)?;
        let name = body["name"].as_str().unwrap_or("").trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(ApiError::bad("Key name must be 1-64 characters"));
        }
        let scope = match body["scope"].as_str().unwrap_or("full") {
            "full" => KeyScope::Full,
            "read_only" | "read" => KeyScope::ReadOnly,
            _ => return Err(ApiError::bad("scope must be 'full' or 'read_only'")),
        };
        let expires_at = match body.get("expires_days").and_then(Value::as_u64) {
            None | Some(0) => String::new(),
            Some(days) if days <= 3650 => {
                (chrono::Utc::now() + chrono::Duration::days(days as i64)).to_rfc3339()
            }
            Some(_) => return Err(ApiError::bad("expires_days must be 0-3650")),
        };
        let owner = match body["user"]
            .as_str()
            .map(str::trim)
            .filter(|u| !u.is_empty())
        {
            Some(other) if other != actor.username => {
                if !self.can_manage_all_keys(actor).await {
                    return Err(ApiError::forbidden(
                        "Only user managers may create keys for other accounts",
                    ));
                }
                if !self.store.lock().await.users.contains_key(other) {
                    return Err(ApiError::missing("User not found"));
                }
                other.to_owned()
            }
            _ => actor.username.clone(),
        };
        let mut keys = self.api_keys.lock().await;
        if keys.iter().filter(|k| k.user == owner).count() >= MAX_KEYS_PER_USER {
            return Err(ApiError::conflict(format!(
                "At most {MAX_KEYS_PER_USER} API keys per user; revoke one first"
            )));
        }
        let id = loop {
            let candidate = random_alnum(12);
            if !keys.iter().any(|k| k.id == candidate) {
                break candidate;
            }
        };
        let plaintext = format!("{KEY_PREFIX}{id}_{}", random_alnum(40));
        let record = ApiKey {
            id,
            user: owner,
            name: name.to_owned(),
            hash: digest(&plaintext),
            scope,
            created_at: timestamp(),
            last_used: String::new(),
            last_ip: String::new(),
            expires_at,
        };
        let mut next = keys.clone();
        next.push(record.clone());
        self.save_keys(&next).await?;
        *keys = next;
        Ok(json!({"key": plaintext, "record": record.public()}))
    }

    /// Revoke one key. Owners of the key or user managers may revoke.
    pub async fn revoke_api_key(&self, actor: &User, id: &str) -> Result<Value> {
        reject_key_session(actor)?;
        let manage_all = self.can_manage_all_keys(actor).await;
        let mut keys = self.api_keys.lock().await;
        let Some(index) = keys
            .iter()
            .position(|k| k.id == id && (manage_all || k.user == actor.username))
        else {
            return Err(ApiError::missing("API key not found"));
        };
        let mut next = keys.clone();
        let removed = next.remove(index);
        self.save_keys(&next).await?;
        *keys = next;
        Ok(json!({"success": true, "message": format!("Revoked '{}'", removed.name)}))
    }

    /// Drop every key a deleted account owned.
    pub(super) async fn purge_api_keys_for(&self, username: &str) {
        let mut keys = self.api_keys.lock().await;
        if !keys.iter().any(|k| k.user == username) {
            return;
        }
        let next: Vec<ApiKey> = keys
            .iter()
            .filter(|k| k.user != username)
            .cloned()
            .collect();
        match self.save_keys(&next).await {
            Ok(()) => *keys = next,
            Err(e) => {
                tracing::error!(%e, user = username, "could not purge API keys of deleted user")
            }
        }
    }

    async fn can_manage_all_keys(&self, actor: &User) -> bool {
        self.is_owner(actor).await
            || self
                .permissions(actor)
                .await
                .iter()
                .any(|p| p == "USERS_EDIT")
    }
}

/// Key management is a human, logged-in action: a key cannot mint, list, or
/// revoke keys, so one leaked credential cannot multiply itself.
fn reject_key_session(actor: &User) -> Result<()> {
    if actor.key_scope.is_some() {
        return Err(ApiError::forbidden(
            "API keys cannot manage API keys; sign in to the panel instead",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_shape_is_recognized() {
        let key = format!("omb_{}_{}", "a1B2c3D4e5F6", "x".repeat(40));
        assert_eq!(key_id(&key), Some("a1B2c3D4e5F6"));
        assert!(looks_like_api_key(&key));
        assert!(!looks_like_api_key("eyJhbGciOiJIUzI1NiJ9.e30.sig"));
        assert!(!looks_like_api_key("omb_short_x"));
        assert!(!looks_like_api_key(&format!(
            "omb_a1B2c3D4e5F6_{}",
            "x".repeat(10)
        )));
    }

    #[test]
    fn expiry_is_honored() {
        let mut key = ApiKey {
            id: "a".into(),
            user: "u".into(),
            name: "n".into(),
            hash: String::new(),
            scope: KeyScope::Full,
            created_at: String::new(),
            last_used: String::new(),
            last_ip: String::new(),
            expires_at: String::new(),
        };
        assert!(!key.expired());
        key.expires_at = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        assert!(key.expired());
        key.expires_at = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
        assert!(!key.expired());
    }
}
