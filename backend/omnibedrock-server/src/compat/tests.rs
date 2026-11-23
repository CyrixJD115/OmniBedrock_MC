//! Tests built from real Endstone / BDS log lines.

use super::logs::scan_lines;
use super::packs::Inventory;
use super::parse::{Level, parse_line, split_inline_frames};
use super::rules::classify;
use super::script_api::{fmt_ver, support_from_versions};
use super::statics;
use super::types::{Hint, ModuleSupport, Severity};

const FURNITURE: &str = "[00:00:00] [Server/ERROR]: [Scripting] [Furniture Add-On] InvalidEntityError: Failed to call function 'getComponent' due to Entity being invalid (has the Entity been removed?).    at removeSurroundings (main.js:5)";

#[test]
fn parses_endstone_vanilla_and_panel_prefixes() {
    let e = parse_line(FURNITURE);
    assert_eq!(e.level, Level::Error);
    assert_eq!(e.time, Some("00:00:00"));
    assert!(e.msg.starts_with("[Scripting] [Furniture Add-On]"));
    let v = parse_line("[2026-11-18 03:19:08:123 WARN] Something odd");
    assert_eq!(
        (v.level, v.msg, v.time),
        (Level::Warn, "Something odd", Some("03:19:08"))
    );
    let p = parse_line("[12:17:33 ERROR]: [Scripting] [X] Error: boom");
    assert_eq!(
        (p.level, p.msg),
        (Level::Error, "[Scripting] [X] Error: boom")
    );
    let u = parse_line("plain text");
    assert_eq!(u.level, Level::Unknown);
}

#[test]
fn splits_inline_stack_frames() {
    let (msg, frame) =
        split_inline_frames("InvalidEntityError: bad.    at removeSurroundings (main.js:5)");
    assert_eq!(msg, "InvalidEntityError: bad.");
    let f = frame.unwrap();
    assert_eq!(f.function.as_deref(), Some("removeSurroundings"));
    assert_eq!((f.file.as_deref(), f.line), (Some("main.js"), Some(5)));
}

#[test]
fn classifies_invalid_entity_with_bracketed_pack_names() {
    let msg = parse_line(FURNITURE).msg;
    let c = classify(Level::Error, msg, &[], &[]).unwrap();
    assert_eq!(c.rule, "script.invalid_entity");
    assert_eq!(c.hint, Some(Hint::Name("Furniture Add-On".into())));
    let waila = "[Scripting] [[v5.1] WAILA [BP]] InvalidEntityError: Failed to call function 'runCommand' due to Entity being invalid (has the Entity been removed?).    at <anonymous> (main.js:5)";
    let c = classify(Level::Error, waila, &[], &[]).unwrap();
    assert_eq!(c.hint, Some(Hint::Name("[v5.1] WAILA [BP]".into())));
}

#[test]
fn classifies_content_log_lines() {
    let c = classify(
        Level::Error,
        "[Json] Unknown block during Deferred BlockDescriptor resolution: lv_ce:slimy_grass",
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(
        (c.rule, c.hint),
        (
            "content.unknown_block",
            Some(Hint::Namespace("lv_ce".into()))
        )
    );
    let c = classify(Level::Error, "[FeatureRegistry] RealismCraft | No definition found for feature 'panascais_end:panascais/end/320b'", &[], &[]).unwrap();
    assert_eq!(c.rule, "content.missing_feature");
    let c = classify(Level::Error, "[Json] RealismCraft | actor_definitions | worlds/RealismCraft/behavior_packs/_2Mutant _3Creatures v4.25_r | mutant:mutant_wolf | minecraft:entity | component_groups | minecraft:breedable | breedsWith | child", &[], &[]).unwrap();
    assert_eq!(c.rule, "content.json_schema");
    assert_eq!(
        c.hint,
        Some(Hint::Folder("_2Mutant _3Creatures v4.25_r".into()))
    );
    let c = classify(Level::Warn, "[Scripting] Custom Command alias [version] already in use. Required to use full name [xp_furniture:version].", &[], &[]).unwrap();
    assert_eq!(
        (c.rule, c.hint),
        (
            "command.alias_conflict",
            Some(Hint::Namespace("xp_furniture".into()))
        )
    );
    let c = classify(Level::Info, "[Scripting] [Minecraft Essentials v2.20.3] [System] Failed to import command: rtp | general | SyntaxError: Could not find export 'Vector3' in module '@minecraft/server'", &[], &[]).unwrap();
    assert_eq!(
        (c.rule, c.severity),
        ("script.missing_export", Severity::Critical)
    );
    assert!(
        classify(
            Level::Info,
            "[Scripting] [BedrockStatistics] Loaded successfully!",
            &[],
            &[]
        )
        .is_none()
    );
}

#[test]
fn groups_a_flood_into_one_issue_with_frames() {
    let mut lines = Vec::new();
    for _ in 0..1000 {
        lines.push(FURNITURE.to_owned());
        lines.push("[00:00:00] [Server/ERROR]:     at <anonymous> (main.js:5)".to_owned());
        lines.push("[00:00:00] [Main/ERROR]: ".to_owned());
    }
    lines.push("[00:00:01] [Main/INFO]: Version: 1.26.65.1".into());
    let scan = scan_lines(&lines, "t");
    assert_eq!(scan.groups.len(), 1);
    let g = scan.groups.values().next().unwrap();
    assert_eq!(g.count, 1000);
    assert_eq!(scan.server_version.as_deref(), Some("1.26.65.1"));
}

fn versions(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn derives_module_support_from_npm_versions() {
    let v = versions(&[
        "2.10.0",
        "2.11.0",
        "2.12.0-beta.1.26.60-stable",
        "2.12.0-beta.1.26.65-stable",
        "2.11.0-beta.1.26.50-stable",
        "2.12.0-rc.1.26.70-preview.26",
    ]);
    let s = support_from_versions("@minecraft/server", &v, [1, 26, 65]);
    assert_eq!(s.max_stable.as_deref(), Some("2.11.0"));
    assert_eq!(s.beta, vec!["2.12.0-beta.1.26.65-stable".to_string()]);
    let old = support_from_versions("@minecraft/server", &v, [1, 26, 55]);
    assert_eq!(old.max_stable.as_deref(), Some("2.10.0"));
    assert_eq!(fmt_ver([1, 2, 3]), "1.2.3");
}

#[test]
fn flags_too_new_modules_and_missing_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let world = dir.path().join("W");
    let bp = world.join("behavior_packs").join("p");
    std::fs::create_dir_all(&bp).unwrap();
    std::fs::write(
        bp.join("manifest.json"),
        r#"{"format_version":2,"header":{"name":"P","uuid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","version":[1,0,0],"min_engine_version":[1,21,0]},
            "modules":[{"type":"script","uuid":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb","version":[1,0,0]}],
            "dependencies":[{"module_name":"@minecraft/server","version":"2.12.0"},{"uuid":"cccccccc-cccc-cccc-cccc-cccccccccccc","version":[1,0,0]}]}"#,
    )
    .unwrap();
    std::fs::write(
        world.join("world_behavior_packs.json"),
        r#"[{"pack_id":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","version":[1,0,0]}]"#,
    )
    .unwrap();
    let inv = Inventory::load(dir.path(), &["W".into()]);
    assert!(inv.packs[0].active);
    let support = vec![ModuleSupport {
        module: "@minecraft/server".into(),
        max_stable: Some("2.11.0".into()),
        beta: vec![],
        source: "npm",
    }];
    let rules: Vec<&str> = statics::check(&inv, Some([1, 26, 65]), &support, Some(true))
        .iter()
        .map(|f| f.rule)
        .collect();
    assert!(rules.contains(&"manifest.module_too_new"), "{rules:?}");
    assert!(rules.contains(&"manifest.missing_dependency"), "{rules:?}");
}

/// `COMPAT_FIXTURE=<dir with server/logs> cargo test -p omnibedrock-server real_logs -- --ignored --nocapture`
#[test]
#[ignore = "needs a real server log fixture"]
fn real_logs() {
    let Ok(root) = std::env::var("COMPAT_FIXTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    for pass in 0..2 {
        let t = std::time::Instant::now();
        let r = rt
            .block_on(super::analyze(
                super::Inputs {
                    server_dir: root.join("server"),
                    cache_dir: root.join("cache"),
                    server_version: None,
                    endstone_version: None,
                    console: vec![],
                },
                super::Options::default(),
            ))
            .unwrap();
        println!(
            "pass {pass}: {:?} files={} bytes={} issues={}",
            t.elapsed(),
            r.summary.files_scanned,
            r.summary.bytes_scanned,
            r.issues.len()
        );
        if pass == 1 {
            for i in &r.issues {
                println!(
                    "{:?} {:>8} {} | pack={:?} hint={:?} | ctx={}",
                    i.severity,
                    i.count,
                    i.title,
                    i.pack.as_ref().map(|p| &p.name),
                    i.hint,
                    i.context.as_deref().map(|c| c.len()).unwrap_or(0)
                );
            }
            if let Some(c) = r.issues[0].context.as_deref() {
                println!("CONTEXT\n{c}");
            }
            for p in r.packs.iter().take(8) {
                println!(
                    "PACK {} {} e={} w={} ev={}",
                    p.status, p.name, p.errors, p.warnings, p.events
                );
            }
            for t in &r.timeline {
                println!(
                    "{} {:?} {:?} lines={} err={}",
                    t.label, t.server_version, t.endstone_version, t.lines, t.errors
                );
            }
            println!("{:?}", r.script_api);
        }
    }
}
