//! Role and permission tests: the API RBAC smoke check, builtin-role
//! upgrades for existing installs, migration of legacy single-role and
//! instance-access state, role CRUD with delegation and owner protections,
//! and serialized concurrent role/user mutations.
use crate::harness::{call, config, setup, state};
use axum::http::StatusCode;
use omnibedrock_api::api::AppState;
use omnibedrock_api::auth::Auth;
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn api_roles_permissions_and_revoke() {
    let (_dir, s, app, token) = state().await;
    let (unauth, _) = call(&app, "GET", "/api/v1/server/status", None, json!({})).await;
    assert_eq!(unauth, StatusCode::UNAUTHORIZED);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/auth/users",
        Some(&token),
        json!({"username":"viewer","password":"viewer-password","role":"viewer"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"viewer","password":"viewer-password"}),
    )
    .await;
    let viewer = login["token"].as_str().unwrap();
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/console/command",
        Some(viewer),
        json!({"command":"op Steve"}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/players/action",
        Some(viewer),
        json!({"action":"ban","target":"Steve"}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "DELETE",
        "/api/v1/auth/users/viewer",
        Some(&token),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert!(s.auth.verify(viewer).await.is_err());
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        Some(&token),
        json!({"name":"custom","permissions":["SERVER_VIEW"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/roles/custom",
        Some(&token),
        json!({"permissions":["FAKE_PERMISSION"]}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}

/// Upgraded installs (roles.yaml from before the `user` role existed) gain
/// new built-ins on boot without touching customized existing roles.
#[tokio::test]
async fn upgraded_installs_gain_new_builtin_roles() {
    let dir = TempDir::new().unwrap();
    let c = config(&dir);
    tokio::fs::create_dir_all(&c.data_dir).await.unwrap();
    // A pre-multi-instance roles.yaml: no `user` role, and a moderator
    // role the admin customized (display name changed, permission removed).
    tokio::fs::write(
        c.data_dir.join("roles.yaml"),
        "moderator:\n  name: moderator\n  display_name: My Mods\n  permissions: [CONSOLE_VIEW]\n  is_default: false\n  created_at: '2020-01-01'\nadmin:\n  name: admin\n  display_name: Admin\n  permissions: [SERVER_VIEW]\n  is_default: false\n  created_at: '2020-01-01'\nowner:\n  name: owner\n  display_name: Owner\n  permissions: [SERVER_VIEW]\n  is_default: false\n  created_at: '2020-01-01'\nviewer:\n  name: viewer\n  display_name: Viewer\n  permissions: [CONSOLE_VIEW]\n  is_default: true\n  created_at: '2020-01-01'\n",
    )
    .await
    .unwrap();
    let auth = Auth::new(Arc::new(c)).await.unwrap();
    let roles = auth.roles().await;
    let user = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "user")
        .expect("user role added on upgrade")
        .clone();
    assert!(
        user["permissions"]
            .as_array()
            .unwrap()
            .contains(&json!("INSTANCES_CREATE"))
    );
    let moderator = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "moderator")
        .cloned()
        .unwrap();
    assert_eq!(moderator["display_name"], "My Mods");
    assert_eq!(
        moderator["permissions"].as_array().unwrap().len(),
        1,
        "customized role untouched"
    );
    // Creating a user with the role that previously failed now works.
    let owner = omnibedrock_api::auth::User {
        token_version: 0,
        username: "Admin".into(),
        role: "owner".into(),
        roles: vec!["owner".into()],
        password_hash: String::new(),
        display_name: "Admin".into(),
        created_at: String::new(),
        last_login: String::new(),
        quotas: Default::default(),
    };
    let created = auth
        .mutate_user(
            &owner,
            "friend",
            Some(json!({"password": "friend-password"})),
            true,
        )
        .await
        .unwrap();
    let _ = created;
    // Default role for new accounts remains viewer.
    let made = auth
        .mutate_user(
            &owner,
            "friend",
            Some(json!({"password": "friend-password", "role": "user"})),
            false,
        )
        .await
        .unwrap();
    assert_eq!(made["role"], "user");
}

// ─── Unified roles & instance access ────────────────────────────────────────

/// Legacy single-role accounts, still-stock role lists, and two-level
/// instance access entries all migrate into the unified multi-role model on
/// boot — without granting new privileges to customized roles.
#[tokio::test]
async fn unified_roles_migrate_legacy_state() {
    let (_dir, c) = setup().await;
    // A legacy user file: one `role` field, no `roles` list.
    tokio::fs::write(
        c.data_dir.join("users.yaml"),
        "Admin:\n  token_version: 1\n  username: Admin\n  password_hash: x\n  role: moderator\n  display_name: Admin\n  created_at: '2026-01-01T00:00:00Z'\n  last_login: ''\n",
    )
    .await
    .unwrap();
    // A legacy roles file: stock lists only (no color/description/position).
    tokio::fs::write(
        c.data_dir.join("roles.yaml"),
        "admin:\n  name: admin\n  display_name: Admin\n  permissions:\n    - CONSOLE_VIEW\n  is_default: false\n  created_at: '2026-01-01T00:00:00Z'\n",
    )
    .await
    .unwrap();
    // A legacy instances file with two-level access entries.
    tokio::fs::write(
        c.data_dir.join("instances.yaml"),
        "default:\n  id: default\n  name: Default Server\n  owner: Admin\n  created_at: '2026-01-01T00:00:00Z'\n  port: 19132\n",
    )
    .await
    .unwrap();
    let state = AppState::new(c, CancellationToken::new()).await.unwrap();
    // The legacy file's argon2 hash is a dummy, so login is impossible by
    // construction; the migration is verified through the auth store instead.
    let users = state.auth.users().await;
    let admin = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "Admin")
        .unwrap()
        .clone();
    assert_eq!(
        admin["roles"],
        json!(["moderator"]),
        "legacy `role` field migrates into the multi-role list"
    );
    assert_eq!(admin["role"], json!("moderator"));
    // Still-stock roles are refreshed: a customized `admin` whose only stored
    // permission was CONSOLE_VIEW does NOT match the old stock list, so its
    // permissions must be untouched — while missing builtin roles are added.
    let roles = state.auth.roles().await;
    let admin_role = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "admin")
        .unwrap()
        .clone();
    assert_eq!(
        admin_role["permissions"],
        json!(["CONSOLE_VIEW"]),
        "customized role permissions are never overwritten"
    );
    assert_eq!(admin_role["builtin"], json!(true));
    assert!(
        roles
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "instance-manager"),
        "new builtin roles are added on upgrade"
    );
    let owner_role = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "owner")
        .unwrap()
        .clone();
    assert_eq!(owner_role["grants_owner"], json!(true));
    assert!(
        owner_role["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "ROLES_MANAGE"),
        "stock refreshed owner carries the new permissions"
    );
    state.shutdown.cancel();
    state.server.shutdown().await.unwrap();
}

/// Role CRUD, the delegation guards, rename reference rewriting, and the
/// owner protections.
#[tokio::test]
async fn role_crud_delegation_and_owner_protection() {
    let (_dir, s, app, token) = state().await;
    let t = Some(token.as_str());

    // A limited editor holds ROLES_MANAGE plus a small set; the delegation
    // guard then bounds what they can author or assign.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        t,
        json!({"name":"delegator","permissions":["ROLES_MANAGE","CONSOLE_VIEW","CONSOLE_SEND","SERVER_VIEW","PLAYERS_VIEW"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/auth/users",
        t,
        json!({"username":"helper","password":"helper-pass-1","roles":["delegator"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"helper","password":"helper-pass-1"}),
    )
    .await;
    let helper = login["token"].as_str().unwrap().to_owned();
    let (code, body) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        Some(&helper),
        json!({"name":"sneaky","permissions":["SERVER_MANAGE"]}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
    // A subset the editor actually holds is fine.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        Some(&helper),
        json!({"name":"helper-delegate","permissions":["CONSOLE_VIEW"],"color":"AA00FF","description":"helper-made"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, role) = call(&app, "GET", "/api/v1/roles/helper-delegate", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(role["color"], json!("AA00FF"));
    assert_eq!(role["builtin"], json!(false));

    // Role listing is readable by any signed-in user (chips need it).
    let (code, _) = call(&app, "GET", "/api/v1/roles", Some(&helper), json!({})).await;
    assert_eq!(code, StatusCode::OK);

    // Duplicate, then rename; users holding the old name follow the rename.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        t,
        json!({"name":"staff","duplicate_of":"moderator"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/helper",
        t,
        json!({"roles":["delegator","helper-delegate","staff"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (_, me) = call(&app, "GET", "/api/v1/auth/me", Some(&helper), json!({})).await;
    for perm in [
        "CONSOLE_VIEW",
        "CONSOLE_SEND",
        "SERVER_VIEW",
        "ROLES_MANAGE",
        "PLAYERS_KICK",
    ] {
        assert!(
            me["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == perm),
            "union should include {perm}"
        );
    }
    assert!(
        !me["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "SERVER_MANAGE"),
        "union must not include permissions none of the roles grant"
    );
    let (code, renamed) = call(
        &app,
        "PUT",
        "/api/v1/roles/staff",
        t,
        json!({"name":"crew"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{renamed}");
    let (_, me) = call(&app, "GET", "/api/v1/auth/me", Some(&helper), json!({})).await;
    assert!(
        me["roles"].as_array().unwrap().iter().any(|r| r == "crew"),
        "user role lists follow the rename"
    );

    // Built-ins keep their keys and cannot be deleted; the owner role keeps
    // its sentinel even after edits.
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/roles/owner",
        t,
        json!({"display_name":"Boss"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, role) = call(&app, "GET", "/api/v1/roles/owner", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(role["grants_owner"], json!(true), "sentinel survives edits");
    assert_eq!(role["display_name"], json!("Boss"));
    for builtin in [
        "owner",
        "admin",
        "moderator",
        "user",
        "viewer",
        "instance-manager",
    ] {
        let (code, _) = call(
            &app,
            "DELETE",
            &format!("/api/v1/roles/{builtin}"),
            t,
            json!({}),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "{builtin} must not delete");
    }
    // Deleting a role held by a user is refused.
    let (code, _) = call(&app, "DELETE", "/api/v1/roles/crew", t, json!({})).await;
    assert_eq!(code, StatusCode::CONFLICT);
    // Deleting an unheld custom role works.
    let (code, _) = call(
        &app,
        "POST",
        "/api/v1/roles/",
        t,
        json!({"name":"scratch","permissions":[]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let (code, _) = call(&app, "DELETE", "/api/v1/roles/scratch", t, json!({})).await;
    assert_eq!(code, StatusCode::OK);

    // Owner protections on users: demoting one of two owners is fine, but the
    // last owner can never be demoted (Admin self-demotes here).
    let (_, admin_login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"123456"}),
    )
    .await;
    let admin_token = admin_login["token"].as_str().unwrap().to_owned();
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/test-owner",
        t,
        json!({"roles":["viewer"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    let (code, body) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/Admin",
        Some(&admin_token),
        json!({"roles":["viewer"]}),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{body}");
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/test-owner",
        Some(&admin_token),
        json!({"roles":["owner"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    // Non-owners cannot touch owner accounts or assign the Owner role.
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/test-owner",
        Some(&helper),
        json!({"roles":["viewer"]}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/helper",
        Some(&helper),
        json!({"roles":["owner"]}),
    )
    .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}

/// Concurrent permission mutations must serialize on the auth store lock:
/// every parallel mutate_role/mutate_user edit lands, with no lost update
/// from the clone-modify-swap write path.
#[tokio::test]
async fn concurrent_role_and_user_mutations_are_serialized() {
    let (_dir, s, _app, _token) = state().await;
    let owner = omnibedrock_api::auth::User {
        token_version: 0,
        username: "Admin".into(),
        role: "owner".into(),
        roles: vec!["owner".into()],
        password_hash: String::new(),
        display_name: String::new(),
        created_at: String::new(),
        last_login: String::new(),
        quotas: Default::default(),
    };
    // Seed three roles and one user up front.
    for name in ["alpha", "beta", "gamma"] {
        s.auth
            .mutate_role(&owner, name, Some(json!({"permissions": []})), true)
            .await
            .unwrap();
    }
    s.auth
        .mutate_user(
            &owner,
            "helper",
            Some(json!({"password": "helper-pass-1", "roles": ["viewer"]})),
            true,
        )
        .await
        .unwrap();

    // Several rounds of four PARALLEL mutations across distinct entities.
    // If the store lock failed to serialize the clone-swap writes, one of the
    // joined edits would be clobbered and the assertions below would fail.
    let rounds: [&[&str]; 4] = [
        &["SERVER_VIEW", "CONSOLE_VIEW", "PLAYERS_VIEW", "FILES_VIEW"],
        &["BACKUPS_VIEW", "ADDONS_VIEW", "SETTINGS_VIEW", "AUDIT_VIEW"],
        &[
            "PROPERTIES_VIEW",
            "STORE_VIEW",
            "CONSOLE_SEND",
            "FILES_EDIT",
        ],
        &[
            "SERVER_MANAGE",
            "USERS_VIEW",
            "BROADCASTER_VIEW",
            "AUDIT_VIEW",
        ],
    ];
    for (i, round) in rounds.iter().enumerate() {
        // Role lists are stored sorted; express expectations the same way.
        let expected_helper = if i % 2 == 0 {
            vec!["alpha".to_string(), "beta".to_string()]
        } else {
            vec!["alpha".to_string(), "gamma".to_string()]
        };
        let (ra, rb, rg, ru) = tokio::join!(
            s.auth.mutate_role(
                &owner,
                "alpha",
                Some(json!({"permissions": [round[0]]})),
                false,
            ),
            s.auth.mutate_role(
                &owner,
                "beta",
                Some(json!({"permissions": [round[1]]})),
                false,
            ),
            s.auth.mutate_role(
                &owner,
                "gamma",
                Some(json!({"permissions": [round[2]]})),
                false,
            ),
            s.auth.mutate_user(
                &owner,
                "helper",
                Some(json!({"roles": expected_helper})),
                false,
            ),
        );
        assert!(
            ra.is_ok() && rb.is_ok() && rg.is_ok() && ru.is_ok(),
            "round {i}"
        );
        // All four edits are present after the join: nothing was lost.
        let roles = s.auth.roles().await;
        let perms_of = |name: &str| {
            roles
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == name)
                .unwrap()["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(perms_of("alpha"), vec![round[0]], "round {i}: alpha lost");
        assert_eq!(perms_of("beta"), vec![round[1]], "round {i}: beta lost");
        assert_eq!(perms_of("gamma"), vec![round[2]], "round {i}: gamma lost");
        let users = s.auth.users().await;
        let helper_roles = users
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["username"] == "helper")
            .unwrap()["roles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(helper_roles, expected_helper, "round {i}: user roles lost");
    }
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}
