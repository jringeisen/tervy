//! Panic reporting never takes GPUI, terminal, database or tracing-writer locks.
//! The hook owns a separate pre-opened log and sends a shutdown request without
//! waiting. Normal unwinding then lets Supervisor::drop join graceful cleanup.
use devdesk_core::supervisor::{Command, SupervisorHandle};
use std::sync::{Arc, OnceLock};

pub struct PanicReporter {
    // Main installs the handle once; the process-wide hook shares its lifetime.
    supervisor: Arc<OnceLock<SupervisorHandle>>,
}
impl PanicReporter {
    pub fn install() -> Self {
        let supervisor: Arc<OnceLock<SupervisorHandle>> = Arc::new(OnceLock::new());
        let hook_supervisor = supervisor.clone();
        let file = crate::logging::panic_file();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(handle) = hook_supervisor.get() {
                handle.send(Command::Shutdown);
            }
            // Payloads may contain terminal data. Log source location instead.
            let location = info.location().map_or_else(
                || "unknown location".into(),
                |location| location.to_string(),
            );
            let message = format!("DevDesk panic at {location}; shutdown requested\n");
            if let Some(file) = &file {
                file.write(message.as_bytes());
            }
            // SAFETY: write to the existing stderr descriptor without acquiring
            // stdio's lock; reporting failure cannot cause another panic.
            unsafe {
                libc::write(2, message.as_ptr().cast(), message.len());
            }
        }));
        Self { supervisor }
    }
    pub fn connect(&self, handle: SupervisorHandle) {
        let _ = self.supervisor.set(handle);
    }
}
