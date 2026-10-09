//! DevDesk's own diagnostics log (implementation-plan.md task 0.7).
//!
//! Writes to `~/Library/Logs/DevDesk/devdesk.log`. The previous log is rotated
//! to `devdesk.log.1` at startup once it reaches 5 MiB. Each writer stops at that cap,
//! so the two files together stay under 10 MiB. Process and terminal output never goes here (ADR-016).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use tracing_subscriber::EnvFilter;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub fn init() {
    // `DEVDESK_LOG=debug` (or any tracing filter) overrides the default.
    let filter = EnvFilter::try_from_env("DEVDESK_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    match open_log_file() {
        Some(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(Mutex::new(CappedWriter::new(file)))
            .init(),
        // Fall back to stderr rather than failing to start.
        None => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init(),
    }
}

fn open_log_file() -> Option<fs::File> {
    let dir = log_dir()?;
    fs::create_dir_all(&dir).ok()?;

    let path = dir.join("devdesk.log");
    if fs::metadata(&path).is_ok_and(|meta| meta.len() >= MAX_LOG_BYTES) {
        let _ = fs::rename(&path, dir.join("devdesk.log.1"));
    }

    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn log_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Logs/DevDesk"))
}

/// Tell tracing each write succeeded even when full, so diagnostics never
/// interfere with app behavior. Rotation occurs on the next launch.
struct CappedWriter {
    file: fs::File,
    remaining: usize,
}
impl CappedWriter {
    fn new(file: fs::File) -> Self {
        let used = file.metadata().map_or(MAX_LOG_BYTES, |meta| meta.len());
        Self {
            file,
            remaining: MAX_LOG_BYTES.saturating_sub(used) as usize,
        }
    }
}
impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.remaining.min(bytes.len());
        self.file.write_all(&bytes[..count])?;
        self.remaining -= count;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
/// Independent append descriptor and atomic byte cap: a panic while tracing
/// holds its writer mutex can still report without trying to acquire that lock.
pub struct PanicFile {
    file: fs::File,
    remaining: AtomicUsize,
}
impl PanicFile {
    pub fn write(&self, bytes: &[u8]) {
        let previous = self
            .remaining
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                Some(remaining.saturating_sub(bytes.len()))
            })
            .unwrap_or(0);
        let count = previous.min(bytes.len());
        let _ = (&self.file).write_all(&bytes[..count]);
    }
}
pub fn panic_file() -> Option<PanicFile> {
    let dir = log_dir()?;
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join("panic.log");
    if fs::metadata(&path).is_ok_and(|meta| meta.len() >= MAX_LOG_BYTES) {
        let _ = fs::rename(&path, dir.join("panic.log.1"));
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let used = file.metadata().ok()?.len();
    Some(PanicFile {
        file,
        remaining: AtomicUsize::new(MAX_LOG_BYTES.saturating_sub(used) as usize),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_log_accepts_writes_without_growing_past_cap() {
        let path = std::env::temp_dir().join(format!("devdesk-log-cap-{}", std::process::id()));
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut writer = CappedWriter {
            file,
            remaining: 12,
        };
        assert_eq!(writer.write(b"first line\n").unwrap(), 11);
        assert_eq!(writer.write(b"second line\n").unwrap(), 12);
        assert_eq!(writer.write(b"third line\n").unwrap(), 11);
        writer.flush().unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 12);
        drop(writer);
        fs::remove_file(path).unwrap();
    }
}
