//! Maps dispatched route segments to audit-log action, detail, and category.
use axum::http::Method;
use serde_json::Value;

use super::helpers::optional_string;

pub(super) fn audit_action(
    p: &[&str],
    method: &Method,
    b: &Value,
) -> (String, String, &'static str) {
    let detail = p.join("/");
    match p {
        ["server", "action"] => (
            format!("server:{}", optional_string(b, "action", "unknown")),
            detail,
            "server",
        ),
        ["server", "backend", "install"] => (
            "server.bds_install".into(),
            optional_string(b, "version", "latest").into(),
            "server",
        ),
        ["server", "backend", "endstone", "install"] => (
            "server.endstone_install".into(),
            optional_string(b, "version", "latest").into(),
            "server",
        ),
        ["server", "backend", "switch"] => (
            format!(
                "server.backend:{}",
                optional_string(b, "backend", "unknown")
            ),
            optional_string(b, "version", "").into(),
            "server",
        ),
        ["broadcaster", "action"] => (
            format!("broadcaster:{}", optional_string(b, "action", "unknown")),
            detail,
            "broadcaster",
        ),
        ["broadcaster", "command"] => (
            "broadcaster.command".into(),
            optional_string(b, "command", "").into(),
            "broadcaster",
        ),
        ["broadcaster", "accounts"] => (
            format!(
                "broadcaster.account_{}",
                optional_string(b, "action", "unknown")
            ),
            optional_string(b, "id", "").into(),
            "broadcaster",
        ),
        ["broadcaster", "config"] => ("broadcaster.config_update".into(), detail, "broadcaster"),
        ["players", "action"] => (
            format!("player:{}", optional_string(b, "action", "unknown")),
            optional_string(b, "target", "").into(),
            "player",
        ),
        ["console", "command"] => (
            "console.command".into(),
            optional_string(b, "command", "").into(),
            "console",
        ),
        ["auth", "users", ..] => (
            format!(
                "auth.user_{}",
                if *method == Method::POST {
                    "create"
                } else if *method == Method::DELETE {
                    "delete"
                } else {
                    "update"
                }
            ),
            detail,
            "auth",
        ),
        ["properties", "raw"] => ("properties.raw_update".into(), detail, "properties"),
        ["properties", key] => (
            format!("properties.update:{key}"),
            optional_string(b, "value", "").into(),
            "properties",
        ),
        ["backups", "create"] => (
            "backup.run".into(),
            optional_string(b, "world", "").into(),
            "backup",
        ),
        ["backups", "settings"] => ("backup.settings_update".into(), detail, "backup"),
        ["backups", "scheduler"] => ("backup.scheduler_update".into(), detail, "backup"),
        ["backups", _, _, "restore-to-world"] => {
            ("backup.restore_to_world".into(), detail, "backup")
        }
        ["backups", ..] => (
            format!("backup.{}", method.as_str().to_lowercase()),
            detail,
            "backup",
        ),
        ["settings", ..] => ("settings.update".into(), detail, "settings"),
        ["addons", action] => (
            format!("addons.{}", action.replace('-', "_")),
            detail,
            "addons",
        ),
        ["addons", "order", ..] => ("addons.reorder".into(), detail, "addons"),
        ["store", "install"] => (
            format!(
                "store.install:{}",
                optional_string(b, "project_slug", "unknown")
            ),
            detail,
            "addons",
        ),
        ["instances", id, "action"] => (
            format!(
                "instance:{}:{}",
                id,
                optional_string(b, "action", "unknown")
            ),
            detail,
            "instances",
        ),
        ["instances", ..] => (
            format!(
                "instance.{}",
                if *method == Method::POST {
                    "create"
                } else if *method == Method::DELETE {
                    "delete"
                } else {
                    "update"
                }
            ),
            detail,
            "instances",
        ),
        ["host"] => ("host.view".into(), detail, "instances"),
        ["files", ..] => (
            format!("files.{}", method.as_str().to_lowercase()),
            detail,
            "files",
        ),
        _ => (
            format!(
                "{}:{}",
                p.first().copied().unwrap_or("general"),
                method.as_str().to_lowercase()
            ),
            detail,
            "general",
        ),
    }
}
