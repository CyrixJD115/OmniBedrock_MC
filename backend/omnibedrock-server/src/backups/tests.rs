use super::archive::{create_zip, prevalidate_archive, restore_archive, verify_archive};
use super::live::Snapshot;
use super::service::is_world_name;
use super::types::BackupRequest;
use std::io::Read;

fn world(dir: &std::path::Path) -> std::path::PathBuf {
    let w = dir.join("worlds").join("W");
    std::fs::create_dir_all(w.join("db")).unwrap();
    std::fs::write(w.join("level.dat"), b"LEVELDAT").unwrap();
    std::fs::write(w.join("db/CURRENT"), b"MANIFEST-000004\n").unwrap();
    std::fs::write(w.join("db/000003.log"), b"0123456789ABCDEF").unwrap();
    std::fs::write(w.join("db/000009.ldb"), b"new-after-hold").unwrap();
    w
}

#[test]
fn live_snapshot_truncates_and_skips_unheld_db_files() {
    let dir = tempfile::tempdir().unwrap();
    let w = world(dir.path());
    let snap: Snapshot = [
        ("level.dat", 8u64),
        ("db/CURRENT", 16),
        ("db/000003.log", 10),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let out = dir.path().join("b.zip");
    let stats = create_zip(&w, &out, &BackupRequest::default(), Some(&snap), &|_, _| {}).unwrap();
    assert_eq!((stats.files, stats.truncated, stats.skipped), (3, 1, 1));
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    let mut log = String::new();
    zip.by_name("W/db/000003.log")
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();
    assert_eq!(log, "0123456789");
    assert!(zip.by_name("W/db/000009.ldb").is_err());
    assert_eq!(verify_archive(&out, "W").unwrap().0, zip.len() as u64);
}

#[test]
fn verify_detects_corruption_and_wrong_world() {
    let dir = tempfile::tempdir().unwrap();
    let w = world(dir.path());
    let out = dir.path().join("b.zip");
    let req = BackupRequest {
        compression: "store".into(),
        ..BackupRequest::default()
    };
    create_zip(&w, &out, &req, None, &|_, _| {}).unwrap();
    assert!(verify_archive(&out, "Other").is_err());
    let mut bytes = std::fs::read(&out).unwrap();
    let at = bytes.windows(8).position(|w| w == b"LEVELDAT").unwrap();
    bytes[at] ^= 0xff;
    std::fs::write(&out, bytes).unwrap();
    let err = verify_archive(&out, "W").unwrap_err().to_string();
    assert!(err.contains("corrupt"), "{err}");
}

#[test]
fn hidden_and_pre_restore_folders_are_not_worlds() {
    assert!(is_world_name("Bedrock level"));
    assert!(!is_world_name(".W.pre-restore-20261123-101010"));
    assert!(!is_world_name("W.bak.20261101-000000.abcd"));
    assert!(!is_world_name(".W.restore.1234"));
}

#[test]
fn prevalidate_accepts_its_own_archive_and_rejects_a_foreign_world() {
    let dir = tempfile::tempdir().unwrap();
    let w = world(dir.path());
    let out = dir.path().join("b.zip");
    create_zip(&w, &out, &BackupRequest::default(), None, &|_, _| {}).unwrap();
    // The archive the panel just wrote for W prevalidates for W…
    assert!(prevalidate_archive(&out, "W").is_ok());
    // …and is refused for any other world before anything is stopped.
    let err = prevalidate_archive(&out, "Other").unwrap_err().to_string();
    assert!(err.contains("different world"), "{err}");
    // restore_archive still enforces the same rule at extraction time.
    let err = restore_archive(&out, &dir.path().join("worlds").join("Other"), "Other")
        .unwrap_err()
        .to_string();
    assert!(err.contains("different world"), "{err}");
    assert!(!dir.path().join("worlds").join("Other").exists());
}
