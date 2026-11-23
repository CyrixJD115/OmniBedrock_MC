//! Player endpoints: who is online (reconciled with `list`), the persistent
//! player history, allowlist/permission/ban files, and moderation actions
//! that use console commands while running and edit the files when stopped.
use omnibedrock_core::error::{ApiError, Result};
use omnibedrock_server::{
    instances::Instance,
    players::{self, PERMISSION_LEVELS},
    server::{Backend, Status},
};
use serde_json::{Value, json};
use std::sync::Arc;

use super::helpers::{optional_string, value_string};
use super::state::AppState;
use crate::auth::User;

/// Names from the `list` command, or `None` when the server is not running.
async fn online_names(svc: &Arc<Instance>) -> Result<Option<Vec<String>>> {
    if svc.server.snapshot().status != Status::Running {
        return Ok(None);
    }
    let console = &svc.server.console;
    let mut changes = console.changed.subscribe();
    let mut cursor = Some(console.replay(None, 0).latest);
    svc.server.send_command("list").await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut header_seen = false;
    loop {
        let batch = console.replay(cursor, 256);
        if batch.gap.is_some() {
            return Err(ApiError::conflict(
                "Player response exceeded console retention",
            ));
        }
        for line in batch.lines {
            cursor = Some(line.sequence);
            let lower = line.line.to_lowercase();
            // BDS: "There are 1/10 players online:" then the names on the next
            // line; Endstone prints both on one line.
            if lower.contains("players online") {
                let tail = line
                    .line
                    .rsplit_once(':')
                    .map(|(_, n)| n.trim())
                    .unwrap_or("");
                if tail.is_empty() {
                    header_seen = true;
                    if lower.contains(" 0/") {
                        return Ok(Some(Vec::new()));
                    }
                    continue;
                }
                return Ok(Some(split_names(tail)));
            }
            if header_seen {
                let names = line
                    .line
                    .rsplit_once("]: ")
                    .map(|(_, n)| n)
                    .unwrap_or(&line.line);
                return Ok(Some(split_names(names)));
            }
        }
        if tokio::time::timeout_at(deadline, changes.changed())
            .await
            .is_err()
        {
            return Ok(None);
        }
    }
}

fn split_names(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect()
}

fn permission_of(perms: &[players::PermissionEntry], xuid: Option<&str>) -> &'static str {
    let Some(x) = xuid else { return "member" };
    match perms
        .iter()
        .find(|p| p.xuid == x)
        .map(|p| p.permission.as_str())
    {
        Some("operator") => "operator",
        Some("visitor") => "visitor",
        _ => "member",
    }
}

pub(super) async fn list_players(svc: &Arc<Instance>) -> Result<Value> {
    match online_names(svc).await? {
        Some(names) => svc.players.reconcile(&names),
        None if svc.server.snapshot().status != Status::Running => svc.players.reconcile(&[]),
        None => {}
    }
    let dir = &svc.config.bedrock_server_dir;
    let perms = players::permissions(dir);
    let online: Vec<Value> = svc
        .players
        .list()
        .into_iter()
        .filter(|r| r.online_since.is_some())
        .map(|r| {
            json!({"name": r.name, "xuid": r.xuid, "online_since": r.online_since,
                   "permission": permission_of(&perms, r.xuid.as_deref())})
        })
        .collect();
    Ok(json!({"count": online.len(), "players": online}))
}

/// Everything the Players page shows besides the online list.
pub(super) async fn known(svc: &Arc<Instance>) -> Result<Value> {
    let dir = svc.config.bedrock_server_dir.clone();
    let allow = players::allowlist(&dir);
    let perms = players::permissions(&dir);
    let bans = players::bans(&dir);
    let props = tokio::fs::read_to_string(dir.join("server.properties"))
        .await
        .unwrap_or_default();
    let prop = |k: &str| {
        props.lines().find_map(|l| {
            l.trim()
                .strip_prefix(k)?
                .strip_prefix('=')
                .map(|v| v.trim().to_owned())
        })
    };
    let records = svc.players.list();
    let list: Vec<Value> = records
        .iter()
        .map(|r| {
            json!({
                "name": r.name, "xuid": r.xuid, "first_seen": r.first_seen, "last_seen": r.last_seen,
                "sessions": r.sessions, "playtime_secs": r.playtime_secs, "online_since": r.online_since,
                "permission": permission_of(&perms, r.xuid.as_deref()),
                "allowlisted": allow.iter().any(|a| a.name.eq_ignore_ascii_case(&r.name)),
                "banned": bans.iter().any(|b| b.name.eq_ignore_ascii_case(&r.name)),
            })
        })
        .collect();
    // permissions.json is keyed by xuid; name it when the history knows it.
    let operators: Vec<Value> = perms
        .iter()
        .map(|p| {
            let name = records
                .iter()
                .find(|r| r.xuid.as_deref() == Some(&p.xuid))
                .map(|r| r.name.clone());
            json!({"xuid": p.xuid, "permission": p.permission, "name": name})
        })
        .collect();
    let backend = svc.server.settings.borrow().backend;
    Ok(json!({
        "players": list,
        "allowlist": allow,
        "operators": operators,
        "bans": bans,
        "allowlist_enabled": prop("allow-list").map(|v| v == "true"),
        "max_players": prop("max-players").and_then(|v| v.parse::<u64>().ok()),
        "default_permission": prop("default-player-permission-level"),
        "running": svc.server.snapshot().status == Status::Running,
        "bans_supported": backend == Backend::Endstone,
    }))
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 || name.contains(['\n', '\r', '\0', '"', '\\']) {
        return Err(ApiError::bad("Invalid player name"));
    }
    Ok(())
}

fn clean_text(s: &str) -> Result<String> {
    if s.len() > 256 || s.contains(['\n', '\r', '\0']) {
        return Err(ApiError::bad(
            "Text must be one line, at most 256 characters",
        ));
    }
    Ok(s.trim().to_owned())
}

pub(super) async fn action(
    s: &AppState,
    svc: &Arc<Instance>,
    u: &User,
    b: &Value,
) -> Result<Value> {
    let action = value_string(b, "action")?;
    let permission = match action {
        "kick" | "tell" => "PLAYERS_KICK",
        "ban" | "pardon" => "PLAYERS_BAN",
        "op" | "deop" | "permission" | "allowlist_add" | "allowlist_remove" | "forget" => {
            "PLAYERS_OP"
        }
        _ => return Err(ApiError::bad("Invalid player action")),
    };
    s.require_for(u, permission, svc).await?;
    let target = value_string(b, "target")?.trim();
    check_name(target)?;
    let reason = clean_text(optional_string(b, "reason", ""))?;
    let running = svc.server.snapshot().status == Status::Running;
    let dir = svc.config.bedrock_server_dir.clone();
    let record = svc.players.find(target);
    let xuid = record
        .as_ref()
        .and_then(|r| r.xuid.clone())
        .or_else(|| b["xuid"].as_str().map(str::to_owned));
    let level = match action {
        "op" => Some("operator"),
        "deop" => Some("member"),
        "permission" => {
            let l = value_string(b, "level")?;
            Some(
                *PERMISSION_LEVELS
                    .iter()
                    .find(|p| **p == l)
                    .ok_or_else(|| ApiError::bad("level must be visitor, member or operator"))?,
            )
        }
        _ => None,
    };
    let command = match action {
        "kick" => Some(format!(
            "kick \"{target}\"{}",
            if reason.is_empty() {
                String::new()
            } else {
                format!(" {reason}")
            }
        )),
        "tell" => {
            if reason.is_empty() {
                return Err(ApiError::bad("Message is empty"));
            }
            Some(format!("tell \"{target}\" {reason}"))
        }
        "ban" => Some(format!(
            "ban \"{target}\"{}",
            if reason.is_empty() {
                String::new()
            } else {
                format!(" {reason}")
            }
        )),
        "pardon" => Some(format!("pardon \"{target}\"")),
        // Prefer permissions.json + reload (works for offline players); the
        // op command is the fallback when the xuid is not known yet.
        "op" | "deop" if running && xuid.is_none() => Some(format!("{action} \"{target}\"")),
        "allowlist_add" if running => Some(format!("allowlist add \"{target}\"")),
        "allowlist_remove" if running => Some(format!("allowlist remove \"{target}\"")),
        _ => None,
    };
    let message = if let Some(cmd) = command {
        if !running {
            return Err(ApiError::conflict(format!(
                "The server must be running to {action} a player"
            )));
        }
        svc.server
            .send_command(&cmd)
            .await
            .map_err(|e| ApiError::conflict(e.to_string()))?;
        format!("Sent '{cmd}'")
    } else {
        match action {
            "allowlist_add" | "allowlist_remove" => {
                players::set_allowlisted(&dir, target, xuid.clone(), action == "allowlist_add")
                    .await?;
                format!(
                    "{} {target} {} allowlist.json",
                    if action == "allowlist_add" {
                        "Added"
                    } else {
                        "Removed"
                    },
                    if action == "allowlist_add" {
                        "to"
                    } else {
                        "from"
                    }
                )
            }
            "op" | "deop" | "permission" => {
                let xuid = xuid.ok_or_else(|| {
                    ApiError::conflict(format!("{target}'s xuid is unknown. They must join once, or start the server and use the console"))
                })?;
                let level = level.unwrap_or("member");
                players::set_permission(&dir, &xuid, level).await?;
                if running {
                    // permissions.json is only re-read on reload.
                    let _ = svc.server.send_command("permission reload").await;
                }
                format!(
                    "{target} is now {level}{}",
                    if running {
                        ""
                    } else {
                        " (applies on next start)"
                    }
                )
            }
            "forget" => {
                if !svc.players.forget(target) {
                    return Err(ApiError::missing("No history for that player"));
                }
                format!("Forgot {target}'s history")
            }
            _ => return Err(ApiError::bad("Invalid player action")),
        }
    };
    s.audit(u, &format!("players.{action}"), target, "player")
        .await
        .ok();
    Ok(json!({"success": true, "message": message}))
}
