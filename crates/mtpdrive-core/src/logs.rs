use crate::model::{LogLevel, LogRecord};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use time::OffsetDateTime;

const MAX_IN_MEMORY: usize = 10_000;
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_FILES: usize = 5;

#[derive(Debug)]
struct FileSink {
    path: PathBuf,
    file: File,
}

/// Thread-safe structured log ring with a small rotating JSON-lines file sink.
#[derive(Debug, Clone)]
pub struct LogStore {
    next_id: Arc<AtomicU64>,
    records: Arc<Mutex<VecDeque<LogRecord>>>,
    sink: Arc<Mutex<Option<FileSink>>>,
}

impl LogStore {
    pub fn new(log_dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(log_dir)?;
        let path = log_dir.join("mtpdrive.log");
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            next_id: Arc::new(AtomicU64::new(1)),
            records: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_IN_MEMORY))),
            sink: Arc::new(Mutex::new(Some(FileSink { path, file }))),
        })
    }

    #[must_use]
    pub fn memory_only() -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(1)),
            records: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_IN_MEMORY))),
            sink: Arc::new(Mutex::new(None)),
        }
    }

    pub fn emit(
        &self,
        level: LogLevel,
        target: impl Into<String>,
        message: impl Into<String>,
    ) -> LogRecord {
        let record = LogRecord {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            unix_millis: OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000,
            level,
            target: target.into(),
            message: message.into(),
        };

        {
            let mut records = self.records.lock();
            if records.len() == MAX_IN_MEMORY {
                let _ = records.pop_front();
            }
            records.push_back(record.clone());
        }

        self.write_record(&record);
        record
    }

    #[must_use]
    pub fn after(&self, after: u64, limit: usize) -> Vec<LogRecord> {
        self.records
            .lock()
            .iter()
            .filter(|record| record.id > after)
            .take(limit.min(MAX_IN_MEMORY))
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.records.lock().clear();
    }

    fn write_record(&self, record: &LogRecord) {
        let mut sink_guard = self.sink.lock();
        let Some(sink) = sink_guard.as_mut() else {
            return;
        };

        if sink
            .file
            .metadata()
            .is_ok_and(|metadata| metadata.len() >= MAX_FILE_BYTES)
        {
            if rotate_files(&sink.path).is_ok()
                && let Ok(file) = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&sink.path)
            {
                sink.file = file;
            }
        }

        if let Ok(mut line) = serde_json::to_vec(record) {
            line.push(b'\n');
            let _ = sink.file.write_all(&line);
            let _ = sink.file.flush();
        }
    }
}

fn rotate_files(path: &Path) -> std::io::Result<()> {
    for index in (1..MAX_FILES).rev() {
        let from = path.with_extension(format!("log.{index}"));
        let to = path.with_extension(format!("log.{}", index + 1));
        if from.exists() {
            std::fs::rename(from, to)?;
        }
    }
    if path.exists() {
        std::fs::rename(path, path.with_extension("log.1"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/logs.rs"]
mod tests;
