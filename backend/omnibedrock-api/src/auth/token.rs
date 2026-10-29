//! Login and bearer tokens: credential authentication (with Argon2 upgrade of
//! legacy hashes and a bounded login semaphore), HS256 JWT issue and verify
//! over the SHA-256-hashed secret, and acting-user resolution including the
//! login-disabled owner mode.

use axum::http::StatusCode;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use omnibedrock_core::error::{ApiError, Result};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::Auth;
use super::password::{hash_password, verify_password};
use super::types::{User, is_owner_in_store, timestamp};

#[derive(Serialize, Deserialize)]
struct Claims {
    #[serde(default)]
    ver: u64,
    sub: String,
    role: String,
    iat: u64,
    exp: u64,
}

impl Auth {
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
}
