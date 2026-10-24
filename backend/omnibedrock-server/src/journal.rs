//! Bounded disk replay spool. A buffered writer serializes publishers and applies pipe
//! back-pressure on slow storage. Readers recover beyond the in-memory history window.
use crate::server::{ConsoleEvent, Replay};
use anyhow::Context;
use std::{collections::VecDeque, path::PathBuf, sync::Arc};
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader, BufWriter},
};
use uuid::Uuid;
const SEGMENT_BYTES: u64 = 4 * 1024 * 1024;
struct Segment {
    path: PathBuf,
    first: u64,
    last: u64,
    size: u64,
    checkpoints: Vec<(u64, u64)>,
}
pub struct Journal {
    root: PathBuf,
    id: Uuid,
    budget: u64,
    segments: VecDeque<Segment>,
    stale: VecDeque<(PathBuf, u64)>,
    writer: Option<BufWriter<fs::File>>,
    bytes: u64,
    failed: bool,
}
impl Journal {
    pub fn new(root: PathBuf, id: Uuid, budget: u64) -> Self {
        Self {
            root,
            id,
            budget,
            segments: VecDeque::new(),
            stale: VecDeque::new(),
            writer: None,
            bytes: 0,
            failed: false,
        }
    }
    async fn init(&mut self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root).await?;
        let mut entries = fs::read_dir(&self.root).await?;
        let mut old = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some((uuid, sequence)) = name.trim_end_matches(".jsonl").split_once('_')
                && name.ends_with(".jsonl")
                && Uuid::parse_str(uuid).is_ok()
                && sequence.parse::<u64>().is_ok()
                && entry.file_type().await?.is_file()
            {
                let metadata = entry.metadata().await?;
                old.push((metadata.modified()?, entry.path(), metadata.len()));
            }
        }
        old.sort_by_key(|e| e.0);
        for (_, path, size) in old {
            self.bytes += size;
            self.stale.push_back((path, size));
        }
        Ok(())
    }
    pub async fn append(&mut self, event: &ConsoleEvent) -> anyhow::Result<()> {
        if self.failed {
            return Ok(());
        }
        let result = self.append_inner(event).await;
        if result.is_err() {
            self.failed = true;
            self.writer = None;
        }
        result
    }
    async fn append_inner(&mut self, event: &ConsoleEvent) -> anyhow::Result<()> {
        if self.writer.is_none() && self.segments.is_empty() {
            self.init().await?;
        }
        let mut data = serde_json::to_vec(event)?;
        data.push(b'\n');
        if self.writer.is_none()
            || self
                .segments
                .back()
                .is_some_and(|s| s.size + data.len() as u64 > SEGMENT_BYTES)
        {
            self.flush().await?;
            let path = self
                .root
                .join(format!("{}_{}.jsonl", self.id, event.sequence));
            let mut options = fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            options.mode(0o600);
            self.writer = Some(BufWriter::with_capacity(
                64 * 1024,
                options.open(&path).await?,
            ));
            self.segments.push_back(Segment {
                path,
                first: event.sequence,
                last: event.sequence,
                size: 0,
                checkpoints: Vec::new(),
            });
        }
        self.writer
            .as_mut()
            .context("journal writer missing")?
            .write_all(&data)
            .await?;
        let segment = self
            .segments
            .back_mut()
            .context("journal segment missing")?;
        if (event.sequence - segment.first).is_multiple_of(256) {
            segment.checkpoints.push((event.sequence, segment.size));
        }
        segment.last = event.sequence;
        segment.size += data.len() as u64;
        self.bytes += data.len() as u64;
        while self.bytes > self.budget {
            if let Some((path, size)) = self.stale.pop_front() {
                fs::remove_file(path).await?;
                self.bytes -= size;
            } else if self.segments.len() > 1 {
                let segment = self.segments.pop_front().context("segment missing")?;
                fs::remove_file(segment.path).await?;
                self.bytes -= segment.size;
            } else {
                break;
            }
        }
        Ok(())
    }
    pub async fn flush(&mut self) -> anyhow::Result<()> {
        if let Some(writer) = &mut self.writer
            && let Err(e) = writer.flush().await
        {
            self.failed = true;
            self.writer = None;
            return Err(e.into());
        }
        Ok(())
    }
    pub async fn replay(&mut self, after: u64, limit: usize) -> anyhow::Result<Option<Replay>> {
        if self.failed || self.segments.is_empty() {
            return Ok(None);
        }
        self.flush().await?;
        let first = self.segments.front().context("segment missing")?.first;
        let latest = self.segments.back().context("segment missing")?.last;
        let gap = if after.saturating_add(1) < first {
            Some((after + 1, first - 1))
        } else {
            None
        };
        let mut lines = Vec::new();
        for segment in &self.segments {
            if segment.last <= after {
                continue;
            }
            let mut reader = BufReader::new(fs::File::open(&segment.path).await?);
            let offset = segment
                .checkpoints
                .iter()
                .rev()
                .find(|(sequence, _)| *sequence <= after.saturating_add(1))
                .map_or(0, |(_, offset)| *offset);
            reader.seek(std::io::SeekFrom::Start(offset)).await?;
            let mut text = String::new();
            loop {
                text.clear();
                if reader.read_line(&mut text).await? == 0 {
                    break;
                }
                let event: ConsoleEvent = serde_json::from_str(&text)?;
                if event.sequence > after {
                    lines.push(Arc::new(event));
                    if lines.len() >= limit {
                        return Ok(Some(Replay { lines, gap, latest }));
                    }
                }
            }
        }
        Ok(Some(Replay { lines, gap, latest }))
    }
}
