//! A blocking accept thread has no idle timer. At most 16 short-lived readers
//! parse compact messages, then try-send into a bounded queue. The actor alone
//! authenticates launch generations/tokens and applies per-agent rate limits.
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookEvent {
    pub agent_id: String,
    pub launch: u64,
    pub token: String,
    pub event: String,
    pub session_id: Option<String>,
    pub source: Option<String>,
    pub notification_type: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_name: Option<String>,
    #[serde(default)]
    pub stop_hook_active: bool,
}
impl HookEvent {
    pub fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.agent_id).is_ok()
            && self.launch > 0
            && self.token.len() == 36
            && (crate::agent::EVENTS.contains(&self.event.as_str())
                || self.event == "agent-turn-complete")
            && self
                .session_id
                .as_ref()
                .is_none_or(|id| uuid::Uuid::parse_str(id).is_ok())
            && [
                &self.source,
                &self.notification_type,
                &self.tool_use_id,
                &self.tool_name,
            ]
            .iter()
            .all(|field| field.as_ref().is_none_or(|s| s.len() <= 256))
    }
}
pub struct Server {
    pub path: PathBuf,
    pub events: mpsc::Receiver<HookEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    pub fn bind(support: &Path, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let mut directory = support.join("run");
        if directory.join("devdesk.sock").as_os_str().len() >= 104 {
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            support.hash(&mut hash);
            directory = PathBuf::from("/tmp").join(format!(
                "devdesk-{}-{:x}",
                unsafe { libc::geteuid() },
                hash.finish()
            ));
        }
        std::fs::create_dir_all(&directory)?;
        let metadata = std::fs::symlink_metadata(&directory)?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::other("Hook directory is not owned by this user"));
        }
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        let path = directory.join("devdesk.sock");
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err(io::Error::other(
                    "Refusing to replace a non-owned hook socket",
                ));
            }
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let active = Arc::new(AtomicUsize::new(0));
        let (sender, events) = mpsc::sync_channel(128);
        let wake = Arc::new(wake);
        let thread = std::thread::Builder::new()
            .name("devdesk-hooks".into())
            .spawn(move || {
                for connection in listener.incoming() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(connection) = connection else { continue };
                    if active.load(Ordering::Acquire) >= 16 {
                        continue;
                    }
                    active.fetch_add(1, Ordering::AcqRel);
                    let active = active.clone();
                    let sender = sender.clone();
                    let wake = wake.clone();
                    let worker_active = active.clone();
                    if std::thread::Builder::new()
                        .name("devdesk-hook-read".into())
                        .spawn(move || {
                            match read_message(connection) {
                                Ok(event) => {
                                    if sender.try_send(event).is_ok() {
                                        wake();
                                    }
                                }
                                Err(error) => {
                                    #[cfg(test)]
                                    eprintln!("hook read rejected: {error}");
                                    #[cfg(not(test))]
                                    let _ = error;
                                }
                            }
                            worker_active.fetch_sub(1, Ordering::AcqRel);
                        })
                        .is_err()
                    {
                        active.fetch_sub(1, Ordering::AcqRel);
                    }
                }
            })?;
        Ok(Self {
            path,
            events,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}
fn read_message(mut stream: UnixStream) -> io::Result<HookEvent> {
    let deadline = Instant::now() + Duration::from_millis(250);
    stream.set_nonblocking(true)?;
    let mut bytes = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let n = match stream.read(&mut chunk) {
            Ok(n) => n,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                use std::os::fd::AsRawFd;
                wait(stream.as_raw_fd(), libc::POLLIN, deadline)?;
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Incomplete hook message",
            ));
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > 65536 {
            return Err(io::Error::other("Hook message too large"));
        }
        if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
            let event: HookEvent = serde_json::from_slice(&bytes[..end])?;
            return if event.valid() {
                Ok(event)
            } else {
                Err(io::Error::other("Invalid hook schema"))
            };
        }
    }
}
/// Nonblocking connect and write share one absolute deadline. The helper must
/// never wait behind an unresponsive listener or a full kernel socket buffer.
pub fn send(path: &Path, event: &HookEvent, budget: Duration) -> io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let deadline = Instant::now() + budget;
    let path = path.as_os_str().as_encoded_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.len() >= address.sun_path.len() {
        return Err(io::Error::other("Hook socket path too long"));
    }
    address.sun_family = libc::AF_UNIX as _;
    address.sun_len = std::mem::size_of_val(&address) as _;
    for (out, byte) in address.sun_path.iter_mut().zip(path) {
        *out = *byte as _;
    }
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let no_sigpipe: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            raw,
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&no_sigpipe as *const libc::c_int).cast(),
            std::mem::size_of_val(&no_sigpipe) as _,
        );
    }
    let result = unsafe {
        libc::connect(
            raw,
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as _,
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if !matches!(
            error.raw_os_error(),
            Some(libc::EINPROGRESS) | Some(libc::EAGAIN)
        ) {
            return Err(error);
        }
        wait(raw, libc::POLLOUT, deadline)?;
        let mut error: libc::c_int = 0;
        let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut libc::c_int).cast(),
                &mut size,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
    }
    let mut stream = UnixStream::from(fd);
    let mut bytes = serde_json::to_vec(event)?;
    bytes.push(b'\n');
    if bytes.len() > 65536 {
        return Err(io::Error::other("Hook message too large"));
    }
    let mut offset = 0;
    while offset < bytes.len() {
        match stream.write(&bytes[offset..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => offset += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => wait(raw, libc::POLLOUT, deadline)?,
            Err(e) => return Err(e),
        }
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
    }
    Ok(())
}
pub(crate) fn wait(fd: i32, events: i16, deadline: Instant) -> io::Result<()> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .ok_or(io::ErrorKind::TimedOut)?;
    let mut poll = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    let result = unsafe {
        libc::poll(
            &mut poll,
            1,
            left.as_millis().max(1).min(i32::MAX as u128) as i32,
        )
    };
    if result == 0 {
        Err(io::ErrorKind::TimedOut.into())
    } else if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> HookEvent {
        HookEvent {
            agent_id: uuid::Uuid::new_v4().to_string(),
            launch: 1,
            token: uuid::Uuid::new_v4().to_string(),
            event: "SessionStart".into(),
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            ..Default::default()
        }
    }
    #[test]
    fn slow_clients_and_oversize_do_not_block_valid_hooks_and_permissions_are_private() {
        let folder = std::env::temp_dir().join(format!("devdesk-ipc-{}", std::process::id()));
        let server = Server::bind(&folder, || {}).unwrap();
        assert_eq!(
            std::fs::metadata(server.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&server.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _slow = UnixStream::connect(&server.path).unwrap();
        let mut oversized = UnixStream::connect(&server.path).unwrap();
        let _ = oversized.write_all(&vec![b'x'; 65537]);
        drop(oversized);
        let expected = event();
        send(&server.path, &expected, Duration::from_millis(200)).unwrap();
        let received = server.events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(received.session_id, expected.session_id);
        assert!(
            server
                .events
                .recv_timeout(Duration::from_millis(300))
                .is_err()
        );
        drop(server);
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn schema_rejects_unknown_fields_and_invalid_identity() {
        let mut event = event();
        assert!(event.valid());
        event.launch = 0;
        assert!(!event.valid());
        let mut json = serde_json::to_value(event).unwrap();
        json["extra"] = true.into();
        assert!(serde_json::from_value::<HookEvent>(json).is_err());
    }
}
