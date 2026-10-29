//! Account administration on the `Auth` service: user listing and lookup
//! helpers, quota access, bootstrap owner resolution, and the create/edit/
//! delete mutation with owner and permission-delegation guards.

use axum::http::StatusCode;
use omnibedrock_core::error::{ApiError, Result};
use serde_json::{Value, json};

use super::Auth;
use super::password::hash_password;
use super::types::{User, is_owner_in_store, timestamp, union_permissions};

impl Auth {
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
}
