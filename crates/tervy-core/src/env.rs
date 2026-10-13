//! Resolve the GUI's shell environment once (ADR-026). This blocking function
//! belongs on a worker. Later process launches reuse the result with `$SHELL
//! -c`; interactive shell tabs still use the explicit `$SHELL -l` program.

use std::collections::HashMap;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

pub struct ShellEnvironment {
    pub shell: String,
    pub variables: HashMap<String, String>,
    pub warning: Option<String>,
}

impl ShellEnvironment {
    /// Initialization may block other startup workers; it never runs on GPUI.
    /// After this one-shot barrier the environment is immutable and shared.
    pub fn shared() -> Arc<Self> {
        static ENVIRONMENT: OnceLock<Arc<ShellEnvironment>> = OnceLock::new();
        ENVIRONMENT
            .get_or_init(|| Arc::new(Self::resolve()))
            .clone()
    }

    /// Runs on the background executor. No environment values are logged.
    pub fn resolve() -> Self {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|shell| shell.starts_with('/'))
            .unwrap_or_else(|| "/bin/zsh".into());
        match resolve(&shell, Duration::from_secs(3)) {
            Ok(variables) => Self {
                shell,
                variables,
                warning: None,
            },
            Err(error) => {
                let mut variables: HashMap<String, String> = std::env::vars_os()
                    .filter_map(|(key, value)| {
                        Some((key.into_string().ok()?, value.into_string().ok()?))
                    })
                    .collect();
                let path = variables.entry("PATH".into()).or_default();
                if !path.is_empty() {
                    path.push(':');
                }
                path.push_str("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
                Self {
                    shell,
                    variables,
                    warning: Some(format!(
                        "Shell environment unavailable: {error}. Using inherited environment."
                    )),
                }
            }
        }
    }
}

fn resolve(shell: &str, timeout: Duration) -> io::Result<HashMap<String, String>> {
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", "printf '\\0TERVY_ENV\\0'; env -0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let result = (|| {
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing shell stdout"))?;
        // SAFETY: fcntl only changes flags on the pipe we exclusively own.
        unsafe {
            let flags = libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL);
            if flags < 0
                || libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "login shell exceeded 3 s",
                ));
            }
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if output.len() + read > 1024 * 1024 {
                        return Err(io::Error::other("shell environment exceeds 1 MB"));
                    }
                    output.extend_from_slice(&buffer[..read]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        // stdout EOF doesn't imply process exit. Keep the same deadline.
        loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(io::Error::other("login shell exited unsuccessfully"));
                }
                return parse(&output);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "login shell exceeded 3 s",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        // SAFETY: process_group(0) assigned this owned child's PID as the PGID.
        unsafe {
            libc::killpg(child.id() as i32, libc::SIGKILL);
        }
        let _ = child.wait();
    }
    result
}

fn parse(output: &[u8]) -> io::Result<HashMap<String, String>> {
    const MARKER: &[u8] = b"\0TERVY_ENV\0";
    let start = output
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .ok_or_else(|| io::Error::other("login shell did not produce an environment"))?
        + MARKER.len();
    let mut variables = HashMap::new();
    for entry in output[start..].split(|byte| *byte == 0) {
        if let Ok(entry) = std::str::from_utf8(entry)
            && let Some((key, value)) = entry.split_once('=')
            && !key.is_empty()
            && !key.contains('\n')
        {
            variables.insert(key.to_owned(), value.to_owned());
        }
    }
    if !variables.contains_key("PATH") {
        return Err(io::Error::other("shell environment has no PATH"));
    }
    Ok(variables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_records_preserve_newlines_and_equals_in_values() {
        let env = parse(b"startup banner\n\0TERVY_ENV\0PATH=/bin\0VALUE=a=b\nc\0").unwrap();
        assert_eq!(env["VALUE"], "a=b\nc");
        assert_eq!(env["PATH"], "/bin");
        assert!(parse(b"PATH=/bin\0").is_err());
        assert!(parse(b"\0TERVY_ENV\0BROKEN=1\0").is_err());
    }

    #[test]
    fn timeout_is_bounded_even_when_stdout_is_kept_open() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("tervy-env-timeout-{}", std::process::id()));
        std::fs::write(&path, "#!/bin/sh\nsleep 10 &\nwait\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = Instant::now();
        let result = resolve(path.to_str().unwrap(), Duration::from_millis(100));
        std::fs::remove_file(path).unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
