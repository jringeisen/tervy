//! DevDesk's own diagnostics log (implementation-plan.md task 0.7).
//!
//! Writes to `~/Library/Logs/DevDesk/devdesk.log`. The previous log is rotated
//! to `devdesk.log.1` at startup once it passes 5 MB, so the two files together
//! stay under ~10 MB. Process and terminal output never goes here (ADR-016).

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub fn init() {
    // `DEVDESK_LOG=debug` (or any tracing filter) overrides the default.
    let filter = EnvFilter::try_from_env("DEVDESK_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    match open_log_file() {
        Some(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(Mutex::new(file))
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
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
        let _ = fs::rename(&path, dir.join("devdesk.log.1"));
    }

    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn log_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Logs/DevDesk"))
}
