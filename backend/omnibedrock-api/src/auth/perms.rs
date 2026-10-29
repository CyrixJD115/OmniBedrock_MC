//! Permission resolution and enforcement on the `Auth` service: owner
//! detection, effective global permissions, per-instance permissions, the
//! `require`/`require_instance` gates, instance access checks and the `me`
//! projection.

use omnibedrock_core::error::{ApiError, Result};
use serde_json::{Value, json};

use super::Auth;
use super::consts::PERMISSIONS;
use super::types::{User, is_owner_in_store, union_permissions};

impl Auth {
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
        // Authenticated-only: the seeded default password being active is a
        // first-run state the panel should warn about, but it must not be
        // public pre-login (it would confirm the default pair works).
        v["default_credentials"] = json!(self.default_credentials_active());
        v
    }
}
