//! Built-in role seeding: the stock role definitions, the permission lists
//! stock roles shipped before the unified role system, and the idempotent
//! seeding pass run when the auth store loads.

use std::collections::BTreeMap;

use super::consts::PERMISSIONS;
use super::types::{Role, timestamp};

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
pub(super) fn seed_roles(store: &mut BTreeMap<String, Role>) {
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
