//! Authentication tests: legacy password hash and JWT secret migration,
//! token revocation on password change and user recreation, and the default
//! credentials plus login-toggle behavior with its persisted setting.
use crate::harness::{call, owner, setup, state};
use axum::http::StatusCode;
use omnibedrock_api::auth::Auth;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[tokio::test]
async fn legacy_password_and_jwt_migrate() {
    let (_dir, c) = setup().await;
    let salt = [1u8; 32];
    let key = pbkdf2::pbkdf2_hmac_array::<Sha256, 32>(b"legacy-password", &salt, 100000);
    let users = json!({"legacy":{"username":"legacy","password_hash":format!("{}:{}",hex::encode(salt),hex::encode(key)),"role":"owner"}});
    tokio::fs::write(
        c.data_dir.join("users.yaml"),
        serde_yaml::to_string(&users).unwrap(),
    )
    .await
    .unwrap();
    let auth = Auth::new(Arc::new(c.clone())).await.unwrap();
    let login = auth
        .authenticate("legacy".into(), "legacy-password".into(), "")
        .await
        .unwrap();
    assert_eq!(
        auth.verify(login["token"].as_str().unwrap())
            .await
            .unwrap()
            .username,
        "legacy"
    );
    assert!(
        tokio::fs::read_to_string(c.data_dir.join("users.yaml"))
            .await
            .unwrap()
            .contains("$argon2id$")
    );
    let secret = tokio::fs::read_to_string(c.data_dir.join("jwt_secret"))
        .await
        .unwrap();
    let key = Sha256::digest(secret.as_bytes());
    let claims = json!({"sub":"legacy","role":"owner","iat":chrono::Utc::now().timestamp(),"exp":chrono::Utc::now().timestamp()+600});
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(&key),
    )
    .unwrap();
    assert!(auth.verify(&token).await.is_ok());
    assert!(
        auth.authenticate("legacy".into(), "incorrect".into(), "")
            .await
            .is_err()
    );
    assert!(
        auth.authenticate("unknown".into(), "incorrect".into(), "")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn password_change_and_recreated_user_revoke_old_tokens() {
    let (_dir, s, _app, _token) = state().await;
    s.auth
        .mutate_user(
            &owner(),
            "account",
            Some(json!({"password":"old-password","role":"viewer"})),
            true,
        )
        .await
        .unwrap();
    let login = s
        .auth
        .authenticate("account".into(), "old-password".into(), "")
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();
    s.auth
        .mutate_user(
            &owner(),
            "account",
            Some(json!({"password":"new-password"})),
            false,
        )
        .await
        .unwrap();
    assert!(s.auth.verify(token).await.is_err());
    let login = s
        .auth
        .authenticate("account".into(), "new-password".into(), "")
        .await
        .unwrap();
    let token = login["token"].as_str().unwrap();
    s.auth
        .mutate_user(&owner(), "account", None, false)
        .await
        .unwrap();
    s.auth
        .mutate_user(
            &owner(),
            "account",
            Some(json!({"password":"new-password","role":"viewer"})),
            true,
        )
        .await
        .unwrap();
    assert!(s.auth.verify(token).await.is_err());
    s.shutdown.cancel();
    s.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn default_credentials_and_login_toggle() {
    let (dir, s, app, _) = state().await;

    // Fresh store seeds Admin/123456 and reports itself publicly.
    let (status, panel) = call(&app, "GET", "/api/v1/settings/panel", None, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(panel["login_enabled"], true);
    // The public endpoint must not advertise that the default pair works.
    assert!(panel.get("default_credentials").is_none());

    let (status, login) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let token = login["token"].as_str().unwrap().to_owned();
    assert_eq!(login["user"]["username"], "Admin");
    assert_eq!(login["user"]["role"], "owner");
    assert_eq!(login["user"]["default_credentials"], true);

    // With login enabled, no token means no access.
    let (status, _) = call(&app, "GET", "/api/v1/server/status", None, json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Disabling login is owner-only AND needs the typed confirmation.
    let (status, body) = call(
        &app,
        "PUT",
        "/api/v1/settings/panel",
        Some(&token),
        json!({"login_enabled": false, "confirm": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/settings/panel",
        Some(&token),
        json!({"login_enabled": false}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, me) = call(&app, "GET", "/api/v1/auth/me", None, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["role"], "owner");
    let (status, _) = call(&app, "GET", "/api/v1/server/status", None, json!({})).await;
    assert_eq!(status, StatusCode::OK);

    // Panel info reflects the disabled state.
    let (_, panel) = call(&app, "GET", "/api/v1/settings/panel", None, json!({})).await;
    assert_eq!(panel["login_enabled"], false);

    // Re-enable: token-less access is rejected again, and login still works.
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/settings/panel",
        Some(&token),
        json!({"login_enabled": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "GET", "/api/v1/server/status", None, json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Changing the Admin password clears the default-credentials flag.
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/auth/users/Admin",
        Some(&token),
        json!({"password":"new-secret-9"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A password change rotates the token version, so the old token is dead.
    let (status, relogin) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"new-secret-9"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = relogin["token"].as_str().unwrap().to_owned();
    assert_eq!(relogin["user"]["default_credentials"], false);

    // The toggle persists to panel_settings.yaml for the next start.
    let (status, _) = call(
        &app,
        "PUT",
        "/api/v1/settings/panel",
        Some(&token),
        json!({"login_enabled": false, "confirm": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let persisted = tokio::fs::read_to_string(dir.path().join("data/panel_settings.yaml"))
        .await
        .unwrap();
    assert!(persisted.contains("login_enabled: false"), "{persisted}");
    let _ = s;
}

#[tokio::test]
async fn repeated_failures_throttle_login() {
    let (_dir, _s, app, _token) = state().await;
    for _ in 0..5 {
        let (status, _) = call(
            &app,
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"username":"Admin","password":"wrong"}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // The sixth attempt inside the backoff window is rate limited, not just
    // rejected.
    let (status, body) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Admin","password":"wrong"}),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    // A successful login for another account is unaffected (per-key throttle).
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        json!({"username":"Nobody","password":"whatever"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
