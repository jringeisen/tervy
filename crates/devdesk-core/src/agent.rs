//! Providers build complete argv; the supervisor owns every process. Role is
//! persisted with the record and is the only source of its permission profile.
use crate::{env::ShellEnvironment, ipc::HookEvent};
use devdesk_term::SpawnSpec;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Primary,
    Side,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchProfile {
    Primary,
    ReadOnlySide,
}
impl Role {
    pub fn profile(&self) -> LaunchProfile {
        match self {
            Self::Primary => LaunchProfile::Primary,
            Self::Side => LaunchProfile::ReadOnlySide,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub project: String,
    pub name: String,
    pub cwd: PathBuf,
    pub role: Role,
    pub session_id: Option<String>,
    /// Stable DevDesk identity, never the provider's fork ID. Old records lack it.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub snapshot_at: Option<i64>,
}
#[derive(Clone, Debug)]
pub enum Request {
    New,
    Resume,
    ResumePicker,
    Fork {
        parent_session_id: String,
        question: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Launching,
    Idle,
    Working,
    NeedsApproval,
    Exited,
    TurnFailed,
}
#[derive(Clone, Debug)]
pub struct Info {
    pub record: Record,
    pub status: Status,
}

pub struct Runtime {
    pub info: Info,
    pub request: Request,
    pub token: String,
    /// PermissionRequest omits tool_use_id. Match by tool name when needed;
    /// two parallel requests for the same tool remain inherently ambiguous.
    pending: BTreeSet<String>,
    pub rate_window: std::time::Duration,
    pub rate_count: usize,
}
impl Runtime {
    pub fn new(record: Record, request: Request) -> Self {
        Self {
            info: Info {
                record,
                status: Status::Launching,
            },
            request,
            token: uuid::Uuid::new_v4().to_string(),
            pending: BTreeSet::new(),
            rate_window: std::time::Duration::ZERO,
            rate_count: 0,
        }
    }
    pub fn launching(&mut self) {
        self.info.status = Status::Launching;
        self.pending.clear();
        self.token = uuid::Uuid::new_v4().to_string();
        self.rate_count = 0;
    }
    pub fn exited(&mut self) {
        self.info.status = Status::Exited;
        self.pending.clear();
    }
    pub fn apply(&mut self, event: &HookEvent) {
        if self.info.status == Status::Exited {
            return;
        }
        match event.event.as_str() {
            "SessionStart" => {
                if let Some(id) = &event.session_id {
                    self.info.record.session_id = Some(id.clone());
                }
                if !matches!(event.source.as_deref(), Some("compact")) {
                    self.info.status = Status::Idle;
                }
            }
            "UserPromptSubmit" => {
                self.pending.clear();
                self.info.status = Status::Working;
            }
            "PermissionRequest" => {
                self.pending.insert(
                    event
                        .tool_use_id
                        .clone()
                        .or_else(|| event.tool_name.as_ref().map(|name| format!("tool:{name}")))
                        .unwrap_or_else(|| "pending".into()),
                );
                self.info.status = Status::NeedsApproval;
            }
            "Notification" if event.notification_type.as_deref() == Some("permission_prompt") => {
                if self.pending.is_empty() {
                    self.pending.insert("pending".into());
                }
                self.info.status = Status::NeedsApproval;
            }
            "Notification"
                if event.notification_type.as_deref() == Some("idle_prompt")
                    && self.pending.is_empty() =>
            {
                self.info.status = Status::Idle
            }
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
                if let Some(id) = &event.tool_use_id {
                    self.pending.remove(id);
                }
                if let Some(name) = &event.tool_name {
                    self.pending.remove(&format!("tool:{name}"));
                }
                self.pending.remove("pending");
                self.info.status = if self.pending.is_empty() {
                    Status::Working
                } else {
                    Status::NeedsApproval
                };
            }
            "Stop" => {
                self.info.status = if event.stop_hook_active {
                    Status::Working
                } else if self.pending.is_empty() {
                    Status::Idle
                } else {
                    Status::NeedsApproval
                }
            }
            "StopFailure" => {
                self.pending.clear();
                self.info.status = Status::TurnFailed;
            }
            // SessionEnd never wins over authoritative PTY exit.
            _ => {}
        }
    }
}

pub struct Context<'a> {
    pub record: &'a Record,
    pub request: &'a Request,
    pub launch: u64,
    pub token: &'a str,
    pub socket: &'a Path,
    pub helper: &'a Path,
    pub lines: usize,
}
pub trait AgentProvider {
    fn build_launch(
        &self,
        context: &Context<'_>,
        environment: &ShellEnvironment,
    ) -> Result<SpawnSpec, String>;
    fn check_installed(&self, environment: &ShellEnvironment) -> Result<PathBuf, String>;
}
pub struct ClaudeProvider;
pub const OBSERVER_PREAMBLE: &str = "You are a read-only observer of a snapshot. The original session is still running in another window. An interrupted tool in this snapshot is still in progress in the original; do not retry, continue or interact with it. Answer the question using the inherited context. You cannot observe the parent’s live status.";

/// Render a persisted wall-clock snapshot in local time; no timer is needed.
pub fn snapshot_label(timestamp: Option<i64>) -> String {
    let Some(timestamp) = timestamp else {
        return "unknown time".into();
    };
    let time = timestamp as libc::time_t;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: both pointers refer to live, correctly sized local allocations.
    if unsafe { libc::localtime_r(&time, local.as_mut_ptr()) }.is_null() {
        return "unknown time".into();
    }
    let local = unsafe { local.assume_init() };
    format!("{:02}:{:02}", local.tm_hour, local.tm_min)
}
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionDenied",
    "Notification",
    "Stop",
    "StopFailure",
    "SessionEnd",
];
impl AgentProvider for ClaudeProvider {
    fn check_installed(&self, environment: &ShellEnvironment) -> Result<PathBuf, String> {
        environment
            .variables
            .get("PATH")
            .into_iter()
            .flat_map(std::env::split_paths)
            .map(|dir| dir.join("claude"))
            .find(|path| {
                use std::os::unix::fs::PermissionsExt;
                path.metadata()
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
            .ok_or_else(|| "Claude Code is not installed on the resolved shell PATH".into())
    }
    fn build_launch(&self, c: &Context<'_>, env: &ShellEnvironment) -> Result<SpawnSpec, String> {
        let profile = c.record.role.profile();
        if profile == LaunchProfile::ReadOnlySide
            && !matches!(c.request, Request::Resume | Request::Fork { .. })
        {
            return Err("Side conversations require an existing fork or parent session".into());
        }
        if profile == LaunchProfile::Primary && matches!(c.request, Request::Fork { .. }) {
            return Err("A fork must persist the read-only Side role".into());
        }
        let mut args = match c.request {
            Request::New => vec![
                "--session-id".into(),
                c.record
                    .session_id
                    .clone()
                    .ok_or("New Claude session needs a UUID")?,
                "--name".into(),
                c.record.name.clone(),
            ],
            Request::Resume => vec![
                "--resume".into(),
                c.record.session_id.clone().ok_or(
                    "This conversation has no session ID; use Resume other Claude session",
                )?,
            ],
            Request::ResumePicker => vec!["--resume".into()],
            Request::Fork {
                parent_session_id, ..
            } => vec![
                "--resume".into(),
                parent_session_id.clone(),
                "--fork-session".into(),
            ],
        };
        let command = format!("{} hook claude", shell_quote(&c.helper.to_string_lossy()));
        let hooks: serde_json::Map<String,serde_json::Value>=EVENTS.iter().map(|name| (name.to_string(),serde_json::json!([{"hooks":[{"type":"command","command":command,"timeout":5}]}]))).collect();
        args.extend([
            "--settings".into(),
            serde_json::json!({"hooks":hooks}).to_string(),
        ]);
        if profile == LaunchProfile::ReadOnlySide {
            args.extend(["--restricted","--strict-mcp-config","--tools=Read,Grep,Glob","--disallowedTools=Edit,Write,NotebookEdit,Bash,Agent,WebFetch,WebSearch,mcp__*,ShareOnboardingGuide","--disable-slash-commands","--permission-mode=dontAsk"].map(str::to_owned));
        }
        if let Request::Fork { question, .. } = c.request {
            args.push(format!("{OBSERVER_PREAMBLE}\n\nQuestion: {question}"));
        }
        let mut variables = env.variables.clone();
        for (key, value) in [
            ("DEVDESK_AGENT_ID", c.record.id.clone()),
            ("DEVDESK_LAUNCH_ID", c.launch.to_string()),
            ("DEVDESK_HOOK_TOKEN", c.token.into()),
            (
                "DEVDESK_HOOK_SOCKET",
                c.socket.to_string_lossy().into_owned(),
            ),
            ("CLAUDE_CODE_NONBLOCKING_STDOUT", "1".into()),
        ] {
            variables.insert(key.into(), value);
        }
        Ok(SpawnSpec {
            program: self.check_installed(env)?.to_string_lossy().into_owned(),
            args,
            cwd: c.record.cwd.clone(),
            env: variables,
            scrollback_lines: c.lines,
        })
    }
}
/// Check on a spawn worker, with a hard deadline. Untested versions warn;
/// they do not block the owner's CLI (I-6). No auth/settings output is logged.
pub fn version_warning(program: &str, environment: &ShellEnvironment) -> Option<String> {
    use std::{
        io::Read,
        os::fd::AsRawFd,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let result = (|| -> Result<String, String> {
        let mut child = Command::new(program)
            .arg("--version")
            .env_clear()
            .envs(&environment.variables)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("version check timed out or failed".into());
                }
            }
        };
        let stdout = child.stdout.take().ok_or("Missing CLI version output")?;
        if unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
            return Err("Cannot read CLI version".into());
        }
        let mut version = String::new();
        stdout
            .take(4096)
            .read_to_string(&mut version)
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("CLI version check failed".into());
        }
        Ok(version.trim().to_string())
    })();
    match result {
        Ok(version) if version.split_whitespace().next() == Some("2.1.295") => None,
        Ok(version) => Some(format!(
            "Untested Claude version {version}; run scripts/cli-probe.sh"
        )),
        Err(error) => Some(format!(
            "Cannot verify Claude version: {error}; run scripts/cli-probe.sh"
        )),
    }
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> Record {
        Record {
            id: "10000000-0000-4000-8000-000000000001".into(),
            project: "p".into(),
            name: "Claude notes".into(),
            cwd: PathBuf::from("/tmp/stored-project"),
            role: Role::Primary,
            parent: None,
            snapshot_at: None,
            session_id: Some("20000000-0000-4000-8000-000000000002".into()),
        }
    }
    fn event(name: &str, tool: Option<&str>) -> HookEvent {
        HookEvent {
            event: name.into(),
            tool_use_id: tool.map(str::to_owned),
            ..Default::default()
        }
    }
    #[test]
    fn claude_complete_argv_and_role_cannot_be_downgraded() {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!("devdesk-argv-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let cli = folder.join("claude");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let environment = ShellEnvironment {
            shell: "/bin/zsh".into(),
            variables: [("PATH".into(), folder.to_string_lossy().into_owned())].into(),
            warning: None,
        };
        let mut record = record();
        for request in [Request::New, Request::Resume, Request::ResumePicker] {
            let c = Context {
                record: &record,
                request: &request,
                launch: 8,
                token: "test-token",
                socket: Path::new("/tmp/devdesk.sock"),
                helper: Path::new("/tmp/Jon's DevDesk"),
                lines: 5000,
            };
            let spec = ClaudeProvider.build_launch(&c, &environment).unwrap();
            let mut expected = match request {
                Request::New => vec![
                    "--session-id",
                    record.session_id.as_deref().unwrap(),
                    "--name",
                    "Claude notes",
                ],
                Request::Resume => vec!["--resume", record.session_id.as_deref().unwrap()],
                Request::ResumePicker => vec!["--resume"],
                Request::Fork { .. } => unreachable!(),
            };
            expected.push("--settings");
            assert_eq!(spec.args[..spec.args.len() - 1], expected);
            let settings: serde_json::Value =
                serde_json::from_str(spec.args.last().unwrap()).unwrap();
            assert_eq!(settings["hooks"].as_object().unwrap().len(), 10);
            for event in [
                "SessionStart",
                "UserPromptSubmit",
                "PermissionRequest",
                "PostToolUse",
                "PostToolUseFailure",
                "PermissionDenied",
                "Notification",
                "Stop",
                "StopFailure",
                "SessionEnd",
            ] {
                assert_eq!(
                    settings["hooks"][event],
                    serde_json::json!([{"hooks":[{"type":"command","command":"'/tmp/Jon'\\''s DevDesk' hook claude","timeout":5}]}])
                );
            }
            assert_eq!(spec.cwd, Path::new("/tmp/stored-project"));
            assert_eq!(spec.env["DEVDESK_AGENT_ID"], record.id);
            assert_eq!(spec.env["DEVDESK_LAUNCH_ID"], "8");
            assert_eq!(spec.env["CLAUDE_CODE_NONBLOCKING_STDOUT"], "1");
        }
        record.role = Role::Side;
        for request in [Request::New, Request::ResumePicker] {
            assert!(
                ClaudeProvider
                    .build_launch(
                        &Context {
                            record: &record,
                            request: &request,
                            launch: 9,
                            token: "t",
                            socket: Path::new("/tmp/s"),
                            helper: Path::new("/tmp/h"),
                            lines: 5000
                        },
                        &environment
                    )
                    .is_err()
            );
        }
        let spec = ClaudeProvider
            .build_launch(
                &Context {
                    record: &record,
                    request: &Request::Resume,
                    launch: 9,
                    token: "t",
                    socket: Path::new("/tmp/s"),
                    helper: Path::new("/tmp/h"),
                    lines: 5000,
                },
                &environment,
            )
            .unwrap();
        assert_eq!(
            &spec.args[4..],
            [
                "--restricted",
                "--strict-mcp-config",
                "--tools=Read,Grep,Glob",
                "--disallowedTools=Edit,Write,NotebookEdit,Bash,Agent,WebFetch,WebSearch,mcp__*,ShareOnboardingGuide",
                "--disable-slash-commands",
                "--permission-mode=dontAsk"
            ]
        );
        let request = Request::Fork {
            parent_session_id: "30000000-0000-4000-8000-000000000003".into(),
            question: "Why this design?".into(),
        };
        record.session_id = None;
        let fork = ClaudeProvider
            .build_launch(
                &Context {
                    record: &record,
                    request: &request,
                    launch: 10,
                    token: "t",
                    socket: Path::new("/tmp/s"),
                    helper: Path::new("/tmp/h"),
                    lines: 5000,
                },
                &environment,
            )
            .unwrap();
        let mut expected = vec![
            "--resume".to_string(),
            "30000000-0000-4000-8000-000000000003".into(),
            "--fork-session".into(),
            "--settings".into(),
            spec.args[3].clone(),
        ];
        expected.extend(spec.args[4..].iter().cloned());
        expected.push(format!("{OBSERVER_PREAMBLE}\n\nQuestion: Why this design?"));
        assert_eq!(fork.args, expected);
        assert_eq!(fork.cwd, record.cwd);
        record.role = Role::Primary;
        assert!(
            ClaudeProvider
                .build_launch(
                    &Context {
                        record: &record,
                        request: &request,
                        launch: 10,
                        token: "t",
                        socket: Path::new("/tmp/s"),
                        helper: Path::new("/tmp/h"),
                        lines: 5000
                    },
                    &environment
                )
                .is_err()
        );
        // M4 records remain readable without a schema migration.
        let mut old = serde_json::to_value(record).unwrap();
        old.as_object_mut().unwrap().remove("parent");
        old.as_object_mut().unwrap().remove("snapshot_at");
        let old: Record = serde_json::from_value(old).unwrap();
        assert!(old.parent.is_none() && old.snapshot_at.is_none());
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn failures_denials_parallel_tools_and_blocked_stop_are_best_effort() {
        let mut runtime = Runtime::new(record(), Request::New);
        runtime.apply(&event("SessionStart", None));
        assert_eq!(runtime.info.status, Status::Idle);
        runtime.apply(&event("UserPromptSubmit", None));
        assert_eq!(runtime.info.status, Status::Working);
        runtime.apply(&event("PermissionRequest", Some("a")));
        runtime.apply(&event("PermissionRequest", Some("b")));
        runtime.apply(&event("PostToolUseFailure", Some("a")));
        assert_eq!(runtime.info.status, Status::NeedsApproval);
        runtime.apply(&event("PermissionDenied", Some("b")));
        assert_eq!(runtime.info.status, Status::Working);
        runtime.apply(&event("PermissionRequest", Some("c")));
        runtime.apply(&event("PostToolUseFailure", Some("c")));
        assert_eq!(runtime.info.status, Status::Working);
        // PermissionRequest's documented payload has tool_name, but no ID.
        // Match parallel requests for distinct tools through that fallback.
        for tool in ["Read", "Bash"] {
            runtime.apply(&HookEvent {
                tool_name: Some(tool.into()),
                ..event("PermissionRequest", None)
            });
        }
        runtime.apply(&HookEvent {
            tool_name: Some("Read".into()),
            ..event("PostToolUse", Some("read-id"))
        });
        assert_eq!(runtime.info.status, Status::NeedsApproval);
        runtime.apply(&HookEvent {
            tool_name: Some("Bash".into()),
            ..event("PostToolUseFailure", Some("bash-id"))
        });
        assert_eq!(runtime.info.status, Status::Working);
        runtime.apply(&event("StopFailure", None));
        assert_eq!(runtime.info.status, Status::TurnFailed);
        runtime.apply(&event("UserPromptSubmit", None));
        runtime.apply(&event("Stop", None));
        assert_eq!(runtime.info.status, Status::Idle);
        // A user's Stop hook may block after ours runs; later tool signals
        // correct the candidate idle state without reading any transcript.
        runtime.apply(&event("PostToolUse", None));
        assert_eq!(runtime.info.status, Status::Working);
        runtime.apply(&HookEvent {
            stop_hook_active: true,
            ..event("Stop", None)
        });
        assert_eq!(runtime.info.status, Status::Working);
        runtime.apply(&event("SessionEnd", None));
        assert_eq!(runtime.info.status, Status::Working);
        runtime.exited();
        runtime.apply(&event("PermissionRequest", None));
        assert_eq!(runtime.info.status, Status::Exited);
    }
}
