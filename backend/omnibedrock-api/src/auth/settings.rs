//! Panel settings owned by the auth service: the login toggle, the CurseForge
//! API key override and its masked hint, the shared `panel_settings.yaml`
//! merge helper, and the public login-page info payload.

use omnibedrock_core::files::atomic_write;
use serde_json::{Value, json};

use super::Auth;
use super::consts::DEFAULT_PANEL_SETTINGS;

impl Auth {
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
}
