//! Streaming log scanner. Reads `logs/latest.log` and rotated
//! `logs/*.log.gz`, assembles multi-line events (JS stack frames, Python
//! tracebacks), classifies each event once per distinct text (memoised:
//! a 700k-line flood is a handful of distinct messages) and aggregates per
//! signature. Results are cached per file; a growing `latest.log` is resumed
//! from the last offset instead of re-read.

use super::parse::{Level, is_continuation, parse_frame, parse_line, strip_codes};
use super::rules::{Classified, classify};
use super::types::Location;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::UNIX_EPOCH;

/// Decompressed bytes read per file at most; the rest is marked truncated.
const FILE_BUDGET: u64 = 384 * 1024 * 1024;
const MEMO_CAP: usize = 8192;
const SAMPLES: usize = 6;

#[derive(Clone, Debug)]
pub struct Group {
    pub c: Arc<Classified>,
    pub count: u64,
    pub samples: Vec<String>,
    pub first: Option<String>,
    pub last: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FileScan {
    pub label: String,
    pub file: String,
    pub mtime: u64,
    len: u64,
    offset: u64,
    head: Vec<u8>,
    pub bytes: u64,
    pub lines: u64,
    pub errors: u64,
    pub warnings: u64,
    pub server_version: Option<String>,
    pub endstone_version: Option<String>,
    pub experiments: Option<Vec<String>>,
    pub plugins: Vec<String>,
    pub groups: HashMap<String, Group>,
    pub truncated: bool,
}

struct Pending {
    level: Level,
    time: Option<String>,
    msg: String,
    frames: Vec<Location>,
}

struct Scanner<'a> {
    scan: &'a mut FileScan,
    memo: HashMap<String, Option<Arc<Classified>>>,
    pending: Option<Pending>,
}

impl Scanner<'_> {
    fn finish(&mut self) {
        let Some(p) = self.pending.take() else { return };
        match p.level {
            Level::Error => self.scan.errors += 1,
            Level::Warn => self.scan.warnings += 1,
            _ => {}
        }
        let key = match p.frames.first() {
            Some(f) => format!("{:?}|{}|{:?}|{:?}", p.level, p.msg, f.function, f.file),
            None => format!("{:?}|{}", p.level, p.msg),
        };
        let classified = if let Some(hit) = self.memo.get(&key) {
            hit.clone()
        } else {
            let c = classify(p.level, &p.msg, &p.frames, &self.scan.plugins).map(Arc::new);
            if self.memo.len() >= MEMO_CAP {
                self.memo.clear();
            }
            self.memo.insert(key, c.clone());
            c
        };
        let Some(c) = classified else { return };
        let when = p.time.map(|t| format!("{} {t}", self.scan.label));
        let group = self
            .scan
            .groups
            .entry(c.key.clone())
            .or_insert_with(|| Group {
                c: c.clone(),
                count: 0,
                samples: Vec::new(),
                first: when.clone(),
                last: None,
            });
        group.count += 1;
        group.last = when.or(group.last.take());
        let sample = c
            .sample
            .clone()
            .unwrap_or_else(|| p.msg.chars().take(400).collect());
        if group.samples.len() < SAMPLES && !group.samples.contains(&sample) {
            group.samples.push(sample);
        }
    }

    fn line(&mut self, raw: &str) {
        self.scan.lines += 1;
        let parsed = parse_line(raw);
        if is_continuation(parsed.msg) {
            if let Some(p) = self.pending.as_mut()
                && p.frames.len() < 4
                && let Some(frame) = parse_frame(parsed.msg)
            {
                p.frames.push(frame);
            }
            return;
        }
        self.finish();
        let msg = if parsed.msg.contains('§') || parsed.msg.contains('\u{1b}') {
            strip_codes(parsed.msg)
        } else {
            parsed.msg.to_owned()
        };
        self.observe(&msg);
        self.pending = Some(Pending {
            level: parsed.level,
            time: parsed.time.map(str::to_owned),
            msg,
            frames: Vec::new(),
        });
    }

    /// Header facts: server/Endstone version, experiments, loaded plugins.
    fn observe(&mut self, msg: &str) {
        if let Some(v) = msg.strip_prefix("Version: ") {
            self.scan
                .server_version
                .get_or_insert_with(|| v.trim().to_owned());
        } else if let Some(v) = msg.strip_prefix("This server is running Endstone version: ") {
            let v = v.split_whitespace().next().unwrap_or_default();
            self.scan
                .endstone_version
                .get_or_insert_with(|| v.to_owned());
        } else if let Some(v) = msg.strip_prefix("Experiment(s) active: ") {
            self.scan.experiments = Some(
                v.split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect(),
            );
        } else if let Some((tag, rest)) = msg.strip_prefix('[').and_then(|m| m.split_once("] "))
            && rest.starts_with("Loading ")
            && !tag.contains(' ')
            && !self.scan.plugins.iter().any(|p| p == tag)
        {
            self.scan.plugins.push(tag.to_owned());
        }
    }
}

fn read_into<R: Read>(reader: R, scanner: &mut Scanner<'_>, budget: u64) -> std::io::Result<u64> {
    let mut reader = BufReader::with_capacity(1 << 20, reader);
    let mut buf = Vec::with_capacity(1024);
    let mut read = 0u64;
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        // Only consume complete lines from a growing file.
        if buf.last() != Some(&b'\n') {
            break;
        }
        read += n as u64;
        scanner.line(&String::from_utf8_lossy(&buf));
        if read >= budget {
            scanner.scan.truncated = true;
            break;
        }
    }
    scanner.finish();
    Ok(read)
}

fn file_meta(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((meta.len(), mtime))
}

fn read_head(path: &Path) -> Vec<u8> {
    let mut head = vec![0u8; 512];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    head.truncate(n);
    head
}

/// Scan one log file, reusing / extending the cached result when possible.
pub fn scan_file(
    path: &Path,
    label: &str,
    cache: &mut HashMap<PathBuf, FileScan>,
) -> Option<FileScan> {
    let (len, mtime) = file_meta(path)?;
    let gz = path.extension().is_some_and(|e| e == "gz");
    if let Some(hit) = cache.get(path)
        && hit.len == len
        && hit.mtime == mtime
    {
        return Some(hit.clone());
    }
    let head = read_head(path);
    let mut scan = match cache.get(path) {
        Some(prev) if !gz && len > prev.offset && prev.head == head && !prev.truncated => {
            prev.clone()
        }
        _ => FileScan {
            label: label.to_owned(),
            file: path.file_name()?.to_string_lossy().into_owned(),
            head: head.clone(),
            ..FileScan::default()
        },
    };
    scan.len = len;
    scan.mtime = mtime;
    scan.label = label.to_owned();
    let mut scanner = Scanner {
        scan: &mut scan,
        memo: HashMap::new(),
        pending: None,
    };
    let result = if gz {
        std::fs::File::open(path).and_then(|f| {
            read_into(
                flate2::read::MultiGzDecoder::new(f),
                &mut scanner,
                FILE_BUDGET,
            )
        })
    } else {
        let offset = scanner.scan.offset;
        std::fs::File::open(path).and_then(|mut f| {
            f.seek(SeekFrom::Start(offset))?;
            read_into(f, &mut scanner, FILE_BUDGET.saturating_sub(offset))
        })
    };
    let read = result.ok()?;
    scan.offset += read;
    scan.bytes += read;
    cache.insert(path.to_path_buf(), scan.clone());
    Some(scan)
}

/// Scan in-memory lines (the live console for servers without a log file).
pub fn scan_lines(lines: &[String], label: &str) -> FileScan {
    let mut scan = FileScan {
        label: label.to_owned(),
        file: "console".into(),
        ..FileScan::default()
    };
    let mut scanner = Scanner {
        scan: &mut scan,
        memo: HashMap::new(),
        pending: None,
    };
    for line in lines {
        scanner.scan.bytes += line.len() as u64 + 1;
        scanner.line(line);
    }
    scanner.finish();
    scan
}

static CACHE: LazyLock<Mutex<HashMap<PathBuf, FileScan>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// (date, is_latest, -rotation part, mtime).
type SortKey = (String, u8, i64, u64);

/// One log file the checker will read, oldest first.
pub struct LogFile {
    pub path: PathBuf,
    pub label: String,
}

/// `logs/*.log.gz` plus `logs/latest.log`, newest `max` files, oldest first.
pub fn discover(server_dir: &Path, max: usize) -> Vec<LogFile> {
    let dir = server_dir.join("logs");
    // Sort key: (date, is_latest, -part, mtime). Rotated `D-2` is older
    // than `D-1`; `latest.log` is always newest. mtime is only a tiebreak, so
    // copied/restored files keep their real order.
    let mut files: Vec<(SortKey, PathBuf, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let Some((_, mtime)) = file_meta(&path) else {
                continue;
            };
            if name == "latest.log" {
                let day = chrono::DateTime::from_timestamp(mtime as i64, 0)
                    .map(|d| {
                        d.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d")
                            .to_string()
                    })
                    .unwrap_or_default();
                files.push(((day.clone(), 1, 0, mtime), path, format!("latest ({day})")));
            } else if let Some(stem) = name
                .strip_suffix(".log.gz")
                .or_else(|| name.strip_suffix(".log"))
            {
                let date = stem.get(..10).unwrap_or(stem).to_owned();
                let part = stem
                    .get(11..)
                    .and_then(|p| p.parse::<i64>().ok())
                    .unwrap_or(0);
                files.push(((date, 0, -part, mtime), path, stem.to_owned()));
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let skip = files.len().saturating_sub(max);
    files
        .into_iter()
        .skip(skip)
        .map(|(_, path, label)| LogFile { path, label })
        .collect()
}

/// Scan the given files through the shared cache; evicts files that vanished.
pub fn scan_all(files: &[LogFile]) -> Vec<FileScan> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|path, _| path.exists());
    files
        .iter()
        .filter_map(|f| scan_file(&f.path, &f.label, &mut cache))
        .collect()
}

/// Drop cached scans under a directory (after logs were deleted).
pub fn forget(dir: &Path) {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|path, _| !path.starts_with(dir));
}
