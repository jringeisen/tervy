//! Fast helper dispatched before GPUI. It projects documented status fields;
//! prompts, tool arguments, results and transcript paths never leave this process.
use crate::ipc::{self, HookEvent};
use serde::Deserialize;
use std::{
    io::{self, Read},
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
#[derive(Default, Deserialize)]
#[serde(default)]
struct Fields {
    hook_event_name: String,
    session_id: Option<String>,
    source: Option<String>,
    notification_type: Option<String>,
    tool_use_id: Option<String>,
    tool_name: Option<String>,
    stop_hook_active: bool,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "thread-id")]
    thread_id: Option<String>,
}

pub fn run(kind: Option<&str>, last: Option<&str>) {
    let _ = report(kind, last);
    if kind == Some("codex-notify")
        && let Some(payload) = last
    {
        chain_notify(payload);
    }
}
/// Preserve the original argv payload exactly. Spawn without waiting: the
/// callback remains in the CLI's owned session, so Stop can still sweep it.
fn chain_notify(payload: &str) {
    let Ok(raw) = std::env::var("TERVY_CODEX_NOTIFY") else {
        return;
    };
    if raw.len() > 65536 {
        return;
    }
    let Ok(args) = serde_json::from_str::<Vec<String>>(&raw) else {
        return;
    };
    let Some(program) = args.first().filter(|program| !program.is_empty()) else {
        return;
    };
    let _ = std::process::Command::new(program)
        .args(&args[1..])
        .arg(payload)
        .env_remove("TERVY_CODEX_NOTIFY")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
fn report(kind: Option<&str>, last: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let fields: Fields = match kind {
        Some("claude") => {
            let stdin = io::stdin();
            let raw = stdin.as_raw_fd();
            if unsafe { libc::fcntl(raw, libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
                return Ok(());
            }
            let reader = Input {
                stdin: stdin.lock(),
                deadline: Instant::now() + Duration::from_millis(200),
            };
            // Deserialize only status fields. Ignored JSON strings are streamed
            // past, so huge assistant messages don't truncate a later session ID.
            serde_json::from_reader(io::BufReader::new(reader.take(64 * 1024 * 1024)))?
        }
        Some("codex-notify") => serde_json::from_str(last.ok_or("Missing notify payload")?)?,
        _ => return Ok(()),
    };
    let event = HookEvent {
        agent_id: std::env::var("TERVY_AGENT_ID")?,
        launch: std::env::var("TERVY_LAUNCH_ID")?.parse()?,
        token: std::env::var("TERVY_HOOK_TOKEN")?,
        event: if kind == Some("codex-notify") {
            fields.kind
        } else {
            fields.hook_event_name
        },
        session_id: fields.session_id.or(fields.thread_id),
        source: fields.source,
        notification_type: fields.notification_type,
        tool_use_id: fields.tool_use_id,
        tool_name: fields.tool_name,
        stop_hook_active: fields.stop_hook_active,
    };
    ipc::send(
        std::path::Path::new(&std::env::var_os("TERVY_HOOK_SOCKET").ok_or("Missing socket")?),
        &event,
        Duration::from_millis(200),
    )?;
    Ok(())
}
struct Input<'a> {
    stdin: io::StdinLock<'a>,
    deadline: Instant,
}
impl Read for Input<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if Instant::now() >= self.deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        loop {
            match self.stdin.read(buffer) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    ipc::wait(self.stdin.as_raw_fd(), libc::POLLIN, self.deadline)?
                }
                result => return result,
            }
        }
    }
}
