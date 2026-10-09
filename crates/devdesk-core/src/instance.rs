//! The lock is held for the process lifetime. A second invocation sends only
//! an activation datagram, never commands or executable content.
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::{
    fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    net::UnixDatagram,
};
use std::path::{Path, PathBuf};
pub struct InstanceGuard {
    _lock: std::fs::File,
    socket: PathBuf,
    listener: Option<std::thread::JoinHandle<()>>,
}
impl InstanceGuard {
    pub fn acquire(directory: &Path) -> io::Result<Option<(Self, async_channel::Receiver<()>)>> {
        std::fs::create_dir_all(directory)?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("devdesk.lock"))?;
        let mut run = directory.join("run");
        if run.join("activate.sock").as_os_str().len() >= 104 {
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            directory.hash(&mut hash);
            run = PathBuf::from(format!(
                "/tmp/devdesk-{}-{:x}",
                unsafe { libc::getuid() },
                hash.finish()
            ));
        }
        let socket = run.join("activate.sock");
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(error);
            }
            // The first process may still be binding its activation socket.
            for _ in 0..5 {
                if UnixDatagram::unbound()?
                    .send_to(b"activate", &socket)
                    .is_ok()
                {
                    return Ok(None);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            return Err(io::Error::other(
                "DevDesk is already running, but its activation socket is unavailable",
            ));
        }
        match std::fs::DirBuilder::new().create(&run) {
            Ok(()) => std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(&run)?;
                if !metadata.is_dir()
                    || metadata.uid() != unsafe { libc::getuid() }
                    || metadata.mode() & 0o077 != 0
                {
                    return Err(io::Error::other(
                        "DevDesk run directory must be owned by you and mode 0700",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        if socket.exists() {
            std::fs::remove_file(&socket)?;
        }
        let receiver = UnixDatagram::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let (events, event_rx) = async_channel::bounded(1);
        let listener = std::thread::Builder::new()
            .name("devdesk-activation".into())
            .spawn(move || {
                let mut bytes = [0u8; 16];
                while let Ok(length) = receiver.recv(&mut bytes) {
                    if &bytes[..length] == b"stop" {
                        break;
                    }
                    if &bytes[..length] == b"activate" {
                        let _ = events.try_send(());
                    }
                }
            })?;
        Ok(Some((
            Self {
                _lock: lock,
                socket,
                listener: Some(listener),
            },
            event_rx,
        )))
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Ok(sender) = UnixDatagram::unbound() {
            let _ = sender.send_to(b"stop", &self.socket);
        }
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_instance_activates_first_and_lock_releases() {
        let directory =
            std::env::temp_dir().join(format!("devdesk-instance-{}", std::process::id()));
        let (guard, activation) = InstanceGuard::acquire(&directory).unwrap().unwrap();
        assert!(InstanceGuard::acquire(&directory).unwrap().is_none());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            if activation.try_recv().is_ok() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        drop(guard);
        let guard = InstanceGuard::acquire(&directory).unwrap().unwrap();
        drop(guard);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
