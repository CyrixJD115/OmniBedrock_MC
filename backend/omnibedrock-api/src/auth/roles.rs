//! Role administration on the `Auth` service: role listing with user counts,
//! single-role lookup, and the create/edit/delete mutation including rename
//! fan-out to users and permission delegation guards.

use omnibedrock_core::error::{ApiError, Result};
use serde_json::{Value, json};

use super::Auth;
use super::consts::PERMISSIONS;
use super::types::{Role, User, is_owner_in_store, timestamp, union_permissions};

impl Auth {
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
