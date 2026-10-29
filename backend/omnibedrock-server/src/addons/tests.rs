//! Tests for addon installation, classification, localization, pack-order
//! syncing, listing, and deletion.

use serde_json::Value;
use std::path::Path;

use super::*;
use std::io::Write;

fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (name, data) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

const BP_MANIFEST: &str = r#"{"format_version":2,"header":{"name":"Cool Behavior","uuid":"11111111-1111-1111-1111-111111111111","version":[1,2,3]},"modules":[{"type":"data","uuid":"22222222-2222-2222-2222-222222222222","version":[1,2,3]}]}"#;
const RP_MANIFEST: &str = r#"{"format_version":2,"header":{"name":"Cool Resource","uuid":"33333333-3333-3333-3333-333333333333","version":[0,1,0]},"modules":[{"type":"resources","uuid":"44444444-4444-4444-4444-444444444444","version":[0,1,0]}]}"#;
// Marketplace-style pack: lang-key name, subpacks, capabilities, metadata.
const MARKETPLACE_RP_MANIFEST: &str = r#"{"format_version":2,"header":{"description":"pack.description","name":"pack.name","uuid":"985f7757-dbea-4f92-94dc-3ce02d14d83b","version":[1,0,86],"min_engine_version":[1,21,120]},"modules":[{"type":"resources","uuid":"2be875b6-3001-408a-a65c-3435d32b6f6c","version":[1,0,86]}],"subpacks":[{"folder_name":"a","name":"128x","memory_tier":12},{"folder_name":"d","name":"16x","memory_tier":2}],"capabilities":["pbr"],"metadata":{"authors":["Spark Universe"]}}"#;
const ICON: &[u8] = b"\x89PNG\r\n\x1a\nfakeicon";

#[test]
fn mcpack_installs_with_type_and_icon() {
    let dir = tempfile::TempDir::new().unwrap();
    let archive = dir.path().join("pack.mcpack");
    write_zip(
        &archive,
        &[
            ("manifest.json", BP_MANIFEST.as_bytes()),
            ("pack_icon.png", ICON),
            ("scripts/main.js", b"console.log(1)"),
        ],
    );
    let installed = install_archive(&archive, dir.path(), "w", None).unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].pack_type, "behavior_packs");
    assert_eq!(installed[0].name, "Cool Behavior");
    assert_eq!(installed[0].version, vec![1, 2, 3]);
    let pack = dir.path().join("w/behavior_packs/Cool Behavior");
    assert!(pack.join("manifest.json").exists());
    assert!(pack.join("scripts/main.js").exists());
    assert!(pack_stats(&pack).2);
}

#[tokio::test]
async fn marketplace_manifest_with_subpacks_installs() {
    // Root-manifest resource pack with lang-key name, subpack folders,
    // capabilities and metadata must install with everything intact.
    let dir = tempfile::TempDir::new().unwrap();
    let archive = dir.path().join("marketplace.mcpack");
    write_zip(
        &archive,
        &[
            ("manifest.json", MARKETPLACE_RP_MANIFEST.as_bytes()),
            ("pack_icon.png", ICON),
            (
                "texts/en_US.lang",
                b"pack.name=RealismCraft 2.6\t#\npack.description=Tiered textures\t#\n",
            ),
            ("subpacks/a/textures/a.png", b"a"),
            ("subpacks/d/textures/d.png", b"d"),
        ],
    );
    let installed = install_archive(&archive, dir.path(), "w", None).unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].pack_type, "resource_packs");
    // Lang-key name falls back to the archive folder ("pack"), and the
    // panel resolves the real name later from texts/en_US.lang.
    let pack = dir
        .path()
        .join("w/resource_packs")
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(installed[0].uuid, "985f7757-dbea-4f92-94dc-3ce02d14d83b");
    assert_eq!(installed[0].version, vec![1, 0, 86]);
    assert!(pack.join("subpacks/a/textures/a.png").exists());
    assert!(pack.join("subpacks/d/textures/d.png").exists());
    // The localization table resolves both keys; marketplace-style
    // trailing `\t#` comments are stripped from the values.
    let texts = read_texts(&pack).await;
    assert_eq!(
        resolve_lang(&texts, Some("pack.name")).as_deref(),
        Some("RealismCraft 2.6")
    );
    assert_eq!(
        resolve_lang(&texts, Some("pack.description")).as_deref(),
        Some("Tiered textures")
    );
}

#[test]
fn lang_values_strip_comment_markers_but_keep_glued_hashes() {
    let langs = parse_lang("a=clean\t#\nb=two words # tail\nc=C# glued\nd=no hash\n# comment\n");
    assert_eq!(langs["a"], "clean");
    assert_eq!(langs["b"], "two words");
    // `#` glued to text stays part of the value.
    assert_eq!(langs["c"], "C# glued");
    assert_eq!(langs["d"], "no hash");
}

#[test]
fn mcaddon_installs_both_types_and_conflicts_suffix() {
    let dir = tempfile::TempDir::new().unwrap();
    let archive = dir.path().join("bundle.mcaddon");
    write_zip(
        &archive,
        &[
            ("bp/manifest.json", BP_MANIFEST.as_bytes()),
            ("rp/manifest.json", RP_MANIFEST.as_bytes()),
        ],
    );
    let installed = install_archive(&archive, dir.path(), "w", None).unwrap();
    assert_eq!(installed.len(), 2);
    assert!(
        dir.path()
            .join("w/behavior_packs/Cool Behavior/manifest.json")
            .exists()
    );
    assert!(
        dir.path()
            .join("w/resource_packs/Cool Resource/manifest.json")
            .exists()
    );
    // Same archive again: names collide and are suffixed, nothing is lost.
    let again = install_archive(&archive, dir.path(), "w", None).unwrap();
    assert_eq!(again.len(), 2);
    assert_ne!(again[0].name, installed[0].name);
}

#[test]
fn traversal_and_missing_manifest_rejected() {
    let dir = tempfile::TempDir::new().unwrap();
    let evil = dir.path().join("evil.zip");
    {
        let file = std::fs::File::create(&evil).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("../escape.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"nope").unwrap();
        zip.finish().unwrap();
    }
    assert!(install_archive(&evil, dir.path(), "w", None).is_err());
    assert!(!dir.path().join("escape.txt").exists());

    let bare = dir.path().join("bare.zip");
    write_zip(&bare, &[("readme.txt", b"no manifest here")]);
    assert!(
        install_archive(&bare, dir.path(), "w", None)
            .unwrap_err()
            .to_string()
            .contains("no manifest.json")
    );
}

#[test]
fn sync_world_packs_rewrites_stale_versions_and_reports_missing() {
    let dir = tempfile::TempDir::new().unwrap();
    let world = dir.path().join("w");
    let bp = world.join("behavior_packs");
    std::fs::create_dir_all(bp.join("Alpha")).unwrap();
    std::fs::create_dir_all(bp.join("Beta")).unwrap();
    std::fs::create_dir_all(world.join("resource_packs/Gamma")).unwrap();
    // Alpha declares version [1,1,6] and depends on a pack missing on disk.
    std::fs::write(
        bp.join("Alpha/manifest.json"),
        r#"{"header":{"uuid":"aaaaaaaa-0000-0000-0000-000000000001","version":[1,1,6]},
            "dependencies":[{"uuid":"aaaaaaaa-0000-0000-0000-0000000000ff","version":[1,0,0]}]}"#,
    )
    .unwrap();
    std::fs::write(
        bp.join("Beta/manifest.json"),
        r#"{"header":{"uuid":"aaaaaaaa-0000-0000-0000-000000000002","version":[2,2,2]}}"#,
    )
    .unwrap();
    std::fs::write(
        world.join("resource_packs/Gamma/manifest.json"),
        r#"{"header":{"uuid":"aaaaaaaa-0000-0000-0000-000000000003","version":[1,0,0]}}"#,
    )
    .unwrap();
    // Marketplace-template style: every order entry pinned to [1,0,0]
    // while the real manifests differ; the missing pack has no folder.
    std::fs::write(
        world.join("world_behavior_packs.json"),
        r#"[{"pack_id":"aaaaaaaa-0000-0000-0000-000000000001","version":[1,0,0]},
            {"pack_id":"aaaaaaaa-0000-0000-0000-000000000002","version":[1,0,0]},
            {"pack_id":"aaaaaaaa-0000-0000-0000-0000000000ff","version":[1,0,0]}]"#,
    )
    .unwrap();
    std::fs::write(
        world.join("world_resource_packs.json"),
        r#"[{"pack_id":"aaaaaaaa-0000-0000-0000-000000000003","version":[1,0,0]}]"#,
    )
    .unwrap();

    let report = sync_world_packs(&world);
    assert_eq!(report.synced, 2, "Alpha and Beta order entries rewritten");
    assert_eq!(report.missing, vec!["aaaaaaaa-0000-0000-0000-0000000000ff"]);
    let orders: Value = serde_json::from_str(
        &std::fs::read_to_string(world.join("world_behavior_packs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(orders[0]["version"], serde_json::json!([1, 1, 6]));
    assert_eq!(orders[1]["version"], serde_json::json!([2, 2, 2]));
    // The missing pack's entry is reported, not silently dropped.
    assert_eq!(orders[2]["pack_id"], "aaaaaaaa-0000-0000-0000-0000000000ff");
    // Idempotent: a second pass finds nothing left to fix.
    let again = sync_world_packs(&world);
    assert_eq!(again.synced, 0);
    assert_eq!(again.missing, vec!["aaaaaaaa-0000-0000-0000-0000000000ff"]);
    // The instance-wide, read-only view reports the same gap.
    assert_eq!(
        missing_dependencies(dir.path()),
        vec!["w: missing pack aaaaaaaa-0000-0000-0000-0000000000ff"]
    );
}

#[test]
fn sync_world_packs_tolerates_missing_order_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let world = dir.path().join("plain");
    std::fs::create_dir_all(&world).unwrap();
    let report = sync_world_packs(&world);
    assert_eq!(report.synced, 0);
    assert!(report.missing.is_empty());
    assert!(missing_dependencies(dir.path()).is_empty());
}

#[test]
fn mcworld_installs_as_world_with_levelname() {
    let dir = tempfile::TempDir::new().unwrap();
    let archive = dir.path().join("world.mcworld");
    write_zip(
        &archive,
        &[
            ("levelname.txt", b"My Test World\n"),
            ("level.dat", b"fake-level-dat"),
            ("db/000003.log", b"log"),
        ],
    );
    assert!(matches!(
        detect_archive(&archive).unwrap(),
        ArchiveKind::World
    ));
    let installed = install_world(&archive, dir.path(), None).unwrap();
    assert_eq!(installed.pack_type, "world");
    assert_eq!(installed.name, "My Test World");
    assert!(dir.path().join("My Test World/level.dat").exists());
    assert!(dir.path().join("My Test World/db/000003.log").exists());
}

#[test]
fn wrapped_mcworld_conflicts_and_forced_names() {
    let dir = tempfile::TempDir::new().unwrap();
    let archive = dir.path().join("wrapped.mcworld");
    write_zip(
        &archive,
        &[
            ("MyWorld/level.dat", b"dat"),
            ("MyWorld/levelname.txt", b"Wrapped"),
        ],
    );
    assert!(matches!(
        detect_archive(&archive).unwrap(),
        ArchiveKind::World
    ));
    let first = install_world(&archive, dir.path(), None).unwrap();
    assert_eq!(first.name, "Wrapped");
    let second = install_world(&archive, dir.path(), None).unwrap();
    assert!(second.name.starts_with("Wrapped-"));
    let forced = install_world(&archive, dir.path(), Some("Other/Name!")).unwrap();
    assert_eq!(forced.name, "Other_Name_");
}

#[test]
fn addon_detection_wins_and_unknown_rejected() {
    let dir = tempfile::TempDir::new().unwrap();
    let mixed = dir.path().join("mixed.zip");
    write_zip(
        &mixed,
        &[
            ("manifest.json", BP_MANIFEST.as_bytes()),
            ("level.dat", b"x"),
        ],
    );
    assert!(matches!(
        detect_archive(&mixed).unwrap(),
        ArchiveKind::Addon
    ));
    let unknown = dir.path().join("unknown.zip");
    write_zip(&unknown, &[("readme.txt", b"nothing here")]);
    assert!(matches!(
        detect_archive(&unknown).unwrap(),
        ArchiveKind::Unknown
    ));
}

#[tokio::test]
async fn delete_strips_uuid_from_order_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let world = dir.path().join("w");
    let pack = world.join("behavior_packs/p");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(world.join("world_behavior_packs.json"),
        r#"[{"pack_id":"11111111-1111-1111-1111-111111111111","version":[1,2,3]},{"pack_id":"other"}]"#)
        .unwrap();
    std::fs::write(
        world.join("world_resource_packs.json"),
        r#"[{"pack_id":"zzz"}]"#,
    )
    .unwrap();
    delete_pack(&pack, &world, "11111111-1111-1111-1111-111111111111")
        .await
        .unwrap();
    assert!(!pack.exists());
    let bp: Value = serde_json::from_str(
        &std::fs::read_to_string(world.join("world_behavior_packs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(bp.as_array().unwrap().len(), 1);
    assert_eq!(bp[0]["pack_id"], "other");
    let rp: Value = serde_json::from_str(
        &std::fs::read_to_string(world.join("world_resource_packs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(rp.as_array().unwrap().len(), 1);
}
