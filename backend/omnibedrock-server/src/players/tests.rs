use super::files::{allowlist, permissions, set_allowlisted, set_permission};
use super::tracker::Players;

#[test]
fn tracks_sessions_and_playtime_from_console_lines() {
    let dir = tempfile::tempdir().unwrap();
    let p = Players::detached(dir.path().join("players.json"));
    p.feed(
        "[12:00:00 INFO]: Player connected: Cyrix JD, xuid: 2535425834438771",
        1000,
    );
    p.feed(
        "[12:00:01 INFO]: Player Spawned: Cyrix JD xuid: 2535425834438771, pfid: 46F6EBACC03838E0",
        1001,
    );
    let r = p.find("cyrix jd").unwrap();
    assert_eq!(
        (r.sessions, r.online_since, r.xuid.as_deref()),
        (1, Some(1000), Some("2535425834438771"))
    );
    assert_eq!(r.pfid.as_deref(), Some("46F6EBACC03838E0"));
    p.feed("[12:10:00 INFO]: Player disconnected: Cyrix JD, xuid: 2535425834438771, pfid: 46F6EBACC03838E0", 1600);
    let r = p.find("Cyrix JD").unwrap();
    assert_eq!(
        (r.online_since, r.playtime_secs, r.last_seen),
        (None, 600, 1600)
    );
    p.feed(
        "[13:00:00 INFO]: Player connected: Cyrix JD, xuid: 2535425834438771",
        5000,
    );
    p.feed("[13:00:30 INFO]: Server stop requested.", 5030);
    let r = p.find("Cyrix JD").unwrap();
    assert_eq!(
        (r.sessions, r.playtime_secs, r.online_since),
        (2, 630, None)
    );
}

#[test]
fn reconcile_sets_the_online_set() {
    let dir = tempfile::tempdir().unwrap();
    let p = Players::detached(dir.path().join("players.json"));
    p.feed("Player connected: A, xuid: 1", 10);
    p.reconcile(&["B".into()]);
    assert!(p.find("A").unwrap().online_since.is_none());
    assert!(p.find("B").unwrap().online_since.is_some());
    assert!(p.forget("A"));
    assert!(p.find("A").is_none());
}

#[tokio::test]
async fn edits_allowlist_and_permissions_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("permissions.json"),
        r#"[{"permission":"operator","xuid":"1"}]"#,
    )
    .unwrap();
    set_allowlisted(dir.path(), "Steve", Some("2".into()), true)
        .await
        .unwrap();
    set_allowlisted(dir.path(), "Alex", None, true)
        .await
        .unwrap();
    set_allowlisted(dir.path(), "steve", None, false)
        .await
        .unwrap();
    let names: Vec<String> = allowlist(dir.path()).into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec!["Alex".to_string()]);
    set_permission(dir.path(), "2", "operator").await.unwrap();
    set_permission(dir.path(), "1", "member").await.unwrap();
    let perms = permissions(dir.path());
    assert_eq!(perms.len(), 1);
    assert_eq!(
        (perms[0].xuid.as_str(), perms[0].permission.as_str()),
        ("2", "operator")
    );
    assert!(set_permission(dir.path(), "2", "admin").await.is_err());
}
