//! Providers build complete argv; the supervisor owns every process. Role is
//! persisted with the record and is the only source of its permission profile.
use crate::{env::ShellEnvironment, ipc::HookEvent};
use devdesk_term::SpawnSpec;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    Claude,
    Codex,
    Gemini,
    OpenCode,
    Aider,
}
impl Provider {
    pub const ALL: [Self; 5] = [
        Self::Claude,
        Self::Codex,
        Self::Gemini,
        Self::OpenCode,
        Self::Aider,
    ];

    pub fn command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::Aider => "aider",
        }
    }

    pub fn from_command(command: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.command() == command)
    }

    /// Providers with integrated session identity and read-only side conversations.
    pub fn tracks_sessions(self) -> bool {
        matches!(self, Self::Claude | Self::Codex)
    }

    pub fn check_installed(self, environment: &ShellEnvironment) -> Result<PathBuf, String> {
        match self {
            Self::Claude => ClaudeProvider.check_installed(environment),
            Self::Codex => crate::codex::CodexProvider.check_installed(environment),
            Self::Gemini | Self::OpenCode | Self::Aider => installed(self.command(), environment),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini CLI",
            Self::OpenCode => "OpenCode",
            Self::Aider => "Aider",
        }
    }
}
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
    #[serde(default)]
    pub provider: Provider,
    #[serde(default)]
    pub launch_args: Vec<String>,
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

/// Parse quoted CLI arguments without running a shell or expanding variables.
pub fn parse_launch_args(input: &str) -> Result<Vec<String>, String> {
    if input.len() > 16_384 || input.contains('\0') {
        return Err("Agent arguments must be at most 16 KiB and contain no null bytes".into());
    }
    shlex::split(input)
        .ok_or_else(|| "Close any open quotes or trailing escape in the arguments".into())
}

pub fn format_launch_args(args: &[String]) -> String {
    shlex::try_join(args.iter().map(String::as_str)).unwrap_or_else(|_| args.join(" "))
}

pub(crate) fn launch_args(record: &Record) -> Result<Vec<String>, String> {
    if record.role == Role::Side {
        // A restored or edited record must never bypass the side profile.
        return Ok(vec![]);
    }
    if record.launch_args.iter().map(String::len).sum::<usize>() > 16_384
        || record.launch_args.iter().any(|arg| arg.contains('\0'))
    {
        return Err("Invalid agent launch arguments".into());
    }
    Ok(record.launch_args.clone())
}
#[derive(Clone, Debug)]
pub enum Request {
    New,
    Prompt(String),
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
    Running,
    Idle,
    Working,
    NeedsApproval,
    Exited,
    TurnFailed,
    Attention,
    ActiveInferred,
}
#[derive(Clone, Debug)]
pub struct Info {
    pub record: Record,
    pub status: Status,
}

pub struct Runtime {
    /// The configured public OSC title contains a truncated current-thread UUID.
    /// Internal title-generation turns inherit notify, so an authenticated token
    /// alone does not prove that a notification belongs to the visible thread.
    codex_title_prefix: Option<String>,
    /// PTY titles and authenticated notify messages use independent queues.
    /// Retain at most eight IDs until the first title can correlate them;
    /// never retain notification content or infer an ID from a truncated title.
    codex_pending_ids: VecDeque<String>,
    pub info: Info,
    pub request: Request,
    pub token: String,
    /// PermissionRequest omits tool_use_id. Match by tool name when needed;
    /// two parallel requests for the same tool remain inherently ambiguous.
    pending: BTreeSet<String>,
    pub rate_window: std::time::Duration,
    pub rate_count: usize,
    /// A single deadline exists only while activity is inferred; idle has no timer.
    pub decay_at: Option<std::time::Duration>,
    before_attention: Option<Status>,
}
impl Runtime {
    pub fn new(record: Record, request: Request) -> Self {
        Self {
            codex_title_prefix: None,
            codex_pending_ids: VecDeque::new(),
            info: Info {
                record,
                status: Status::Launching,
            },
            request,
            token: uuid::Uuid::new_v4().to_string(),
            pending: BTreeSet::new(),
            rate_window: std::time::Duration::ZERO,
            rate_count: 0,
            decay_at: None,
            before_attention: None,
        }
    }
    pub fn launching(&mut self) {
        self.codex_title_prefix = None;
        self.codex_pending_ids.clear();
        self.info.status = Status::Launching;
        self.pending.clear();
        self.token = uuid::Uuid::new_v4().to_string();
        self.rate_count = 0;
        self.decay_at = None;
        self.before_attention = None;
    }
    pub fn exited(&mut self) {
        self.codex_pending_ids.clear();
        self.info.status = Status::Exited;
        self.pending.clear();
        self.decay_at = None;
        self.before_attention = None;
    }
    /// Returns true when a queued completion is authenticated by this title.
    pub fn observe_title(&mut self, title: &str) -> bool {
        if self.info.record.provider != Provider::Codex || self.info.status == Status::Exited {
            return false;
        }
        let prefix: String = title
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
            .collect();
        if (29..=36).contains(&prefix.len())
            && prefix.bytes().enumerate().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
        {
            let pending = self
                .codex_pending_ids
                .iter()
                .find(|id| id.starts_with(&prefix))
                .cloned();
            self.codex_pending_ids.clear();
            self.codex_title_prefix = Some(prefix);
            if let Some(id) = pending {
                self.codex_completed(id);
                return true;
            }
        }
        false
    }
    fn codex_completed(&mut self, id: String) {
        self.info.record.session_id = Some(id);
        self.info.status = Status::Idle;
        self.decay_at = None;
        self.before_attention = None;
    }
    pub fn input(&mut self, enter: bool, now: std::time::Duration) {
        if self.info.record.provider != Provider::Codex || self.info.status == Status::Exited {
            return;
        }
        if self.info.status == Status::Attention {
            self.info.status = self.before_attention.take().unwrap_or(Status::Idle);
        }
        if enter {
            self.info.status = Status::ActiveInferred;
            self.decay_at = Some(now + std::time::Duration::from_secs(600));
        }
    }
    pub fn bell(&mut self) {
        if self.info.record.provider == Provider::Codex
            && !matches!(self.info.status, Status::Exited | Status::Attention)
        {
            self.before_attention = Some(self.info.status.clone());
            self.info.status = Status::Attention;
        }
    }
    pub fn expire_inferred(&mut self) {
        self.decay_at = None;
        if self.info.status == Status::ActiveInferred {
            self.info.status = Status::Idle;
        }
        if self.before_attention == Some(Status::ActiveInferred) {
            self.before_attention = Some(Status::Idle);
        }
    }
    pub fn apply(&mut self, event: &HookEvent) {
        if self.info.status == Status::Exited || !self.info.record.provider.tracks_sessions() {
            return;
        }
        if self.info.record.provider == Provider::Codex {
            if event.event == "agent-turn-complete"
                && let Some(id) = &event.session_id
                && uuid::Uuid::parse_str(id).is_ok()
                && !matches!(&self.request, Request::Fork { parent_session_id, .. } if id == parent_session_id)
            {
                if let Some(prefix) = &self.codex_title_prefix {
                    if id.starts_with(prefix) {
                        self.codex_completed(id.clone());
                    }
                } else if !self.codex_pending_ids.contains(id) {
                    if self.codex_pending_ids.len() == 8 {
                        self.codex_pending_ids.pop_front();
                    }
                    self.codex_pending_ids.push_back(id.clone());
                }
            }
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

/// Plain interactive CLIs keep their own authentication and settings. They do
/// not report DevDesk session identities or promise a read-only fork profile.
pub struct InteractiveProvider(pub Provider);
impl AgentProvider for InteractiveProvider {
    fn check_installed(&self, environment: &ShellEnvironment) -> Result<PathBuf, String> {
        installed(self.0.command(), environment)
    }

    fn build_launch(
        &self,
        context: &Context<'_>,
        environment: &ShellEnvironment,
    ) -> Result<SpawnSpec, String> {
        if self.0.tracks_sessions()
            || context.record.provider != self.0
            || context.record.role != Role::Primary
            || !matches!(context.request, Request::New)
        {
            return Err("This agent supports interactive launches only; session resume and side conversations are unavailable".into());
        }
        let mut variables = environment.variables.clone();
        if self.0 == Provider::Gemini {
            variables = hook_environment(context, environment);
            let path = gemini_mcp_settings(context, &variables)?;
            variables.insert(
                "GEMINI_CLI_SYSTEM_DEFAULTS_PATH".into(),
                path.to_string_lossy().into_owned(),
            );
        } else if self.0 == Provider::OpenCode {
            variables = hook_environment(context, environment);
            let mut config: serde_json::Value = variables
                .get("OPENCODE_CONFIG_CONTENT")
                .map(|text| {
                    serde_json::from_str(text)
                        .map_err(|_| "OpenCode inline config must be valid JSON".to_string())
                })
                .transpose()?
                .unwrap_or_else(|| serde_json::json!({}));
            let root = config
                .as_object_mut()
                .ok_or("OpenCode inline config must be an object")?;
            let mcp = root
                .entry("mcp")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .ok_or("OpenCode MCP config must be an object")?;
            if mcp.contains_key("tervy_local") {
                return Err("OpenCode already has an MCP server named tervy_local".into());
            }
            mcp.insert(
                "tervy_local".into(),
                serde_json::json!({
                    "type":"local",
                    "command":[context.helper.to_string_lossy(),"mcp"],
                    "enabled":true,
                    "environment":{
                        "DEVDESK_AGENT_ID":"{env:DEVDESK_AGENT_ID}",
                        "DEVDESK_LAUNCH_ID":"{env:DEVDESK_LAUNCH_ID}",
                        "DEVDESK_HOOK_TOKEN":"{env:DEVDESK_HOOK_TOKEN}",
                        "DEVDESK_MCP_SOCKET":"{env:DEVDESK_MCP_SOCKET}",
                        "DEVDESK_MCP_ROLE":"{env:DEVDESK_MCP_ROLE}"
                    }
                }),
            );
            variables.insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
        }
        Ok(SpawnSpec {
            program: self
                .check_installed(environment)?
                .to_string_lossy()
                .into_owned(),
            args: launch_args(context.record)?,
            cwd: context.record.cwd.clone(),
            env: variables,
            scrollback_lines: context.lines,
        })
    }
}

fn gemini_mcp_settings(
    c: &Context<'_>,
    variables: &std::collections::HashMap<String, String>,
) -> Result<PathBuf, String> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let source = variables
        .get("GEMINI_CLI_SYSTEM_DEFAULTS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("/Library/Application Support/GeminiCli/system-defaults.json")
        });
    let mut settings = match std::fs::metadata(&source) {
        Ok(metadata) => {
            if metadata.len() > 1_048_576 {
                return Err("Gemini system defaults are too large to add Tervy MCP".into());
            }
            serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(&source).map_err(|e| e.to_string())?,
            )
            .map_err(|_| "Gemini system defaults must be valid JSON".to_string())?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(format!("Cannot read Gemini system defaults: {error}")),
    };
    let root = settings
        .as_object_mut()
        .ok_or("Gemini system defaults must be a JSON object")?;
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("Gemini mcpServers must be a JSON object")?;
    if servers.contains_key("tervy_local") {
        return Err("Gemini already has an MCP server named tervy_local".into());
    }
    servers.insert(
        "tervy_local".into(),
        serde_json::json!({
            "command":c.helper.to_string_lossy(), "args":["mcp"], "trust":false,
            "env":{
                "DEVDESK_AGENT_ID":"$DEVDESK_AGENT_ID",
                "DEVDESK_LAUNCH_ID":"$DEVDESK_LAUNCH_ID",
                "DEVDESK_HOOK_TOKEN":"$DEVDESK_HOOK_TOKEN",
                "DEVDESK_MCP_SOCKET":"$DEVDESK_MCP_SOCKET",
                "DEVDESK_MCP_ROLE":"$DEVDESK_MCP_ROLE"
            }
        }),
    );
    let directory = c
        .socket
        .parent()
        .ok_or("Tervy socket directory is missing")?;
    let metadata = std::fs::metadata(directory).map_err(|e| e.to_string())?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o077 != 0 {
        return Err("Tervy socket directory must be private for Gemini settings".into());
    }
    let destination = directory.join("gemini-mcp.json");
    let temporary = directory.join(format!(".gemini-mcp-{}.json", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&settings).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &destination).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map(|()| destination)
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
        installed("claude", environment)
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
            Request::New | Request::Prompt(_) => vec![
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
        args.extend(launch_args(c.record)?);
        let mcp_args = if profile == LaunchProfile::ReadOnlySide {
            vec!["mcp", "--read-only"]
        } else {
            vec!["mcp"]
        };
        let mcp = serde_json::json!({"mcpServers":{"tervy_local":{"type":"stdio","command":c.helper.to_string_lossy(),"args":mcp_args}}});
        args.extend(["--mcp-config".into(), mcp.to_string()]);
        args.extend([
            "--settings".into(),
            serde_json::json!({"hooks":hooks}).to_string(),
        ]);
        if profile == LaunchProfile::ReadOnlySide {
            args.extend(["--restricted","--strict-mcp-config","--tools=Read,Grep,Glob,mcp__tervy_local__list_prompts,mcp__tervy_local__read_prompt","--disallowedTools=Edit,Write,NotebookEdit,Bash,Agent,WebFetch,WebSearch,ShareOnboardingGuide","--disable-slash-commands","--permission-mode=dontAsk"].map(str::to_owned));
        }
        if let Request::Fork { question, .. } = c.request {
            args.push(format!("{OBSERVER_PREAMBLE}\n\nQuestion: {question}"));
        }
        if let Request::Prompt(prompt) = c.request {
            args.push(prompt.clone());
        }
        let mut variables = hook_environment(c, env);
        variables.insert("CLAUDE_CODE_NONBLOCKING_STDOUT".into(), "1".into());
        Ok(SpawnSpec {
            program: self.check_installed(env)?.to_string_lossy().into_owned(),
            args,
            cwd: c.record.cwd.clone(),
            env: variables,
            scrollback_lines: c.lines,
        })
    }
}
pub(crate) fn installed(name: &str, environment: &ShellEnvironment) -> Result<PathBuf, String> {
    environment
        .variables
        .get("PATH")
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name))
        .find(|path| {
            use std::os::unix::fs::PermissionsExt;
            path.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .ok_or_else(|| format!("{name} is not installed on the resolved shell PATH"))
}
pub(crate) fn hook_environment(
    c: &Context<'_>,
    environment: &ShellEnvironment,
) -> std::collections::HashMap<String, String> {
    let mut variables = environment.variables.clone();
    for (key, value) in [
        ("DEVDESK_AGENT_ID", c.record.id.clone()),
        ("DEVDESK_LAUNCH_ID", c.launch.to_string()),
        ("DEVDESK_HOOK_TOKEN", c.token.into()),
        (
            "DEVDESK_HOOK_SOCKET",
            c.socket.to_string_lossy().into_owned(),
        ),
        (
            "DEVDESK_MCP_SOCKET",
            c.socket
                .with_file_name("mcp.sock")
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "DEVDESK_MCP_ROLE",
            if c.record.role == Role::Side {
                "read"
            } else {
                "write"
            }
            .into(),
        ),
    ] {
        variables.insert(key.into(), value);
    }
    variables
}
/// Check on a spawn worker, with a hard deadline. Untested versions warn;
/// they do not block the owner's CLI (I-6). No auth/settings output is logged.
pub fn version_warning(
    provider: Provider,
    program: &str,
    environment: &ShellEnvironment,
) -> Option<String> {
    if !provider.tracks_sessions() {
        return None;
    }
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
        Ok(version)
            if match provider {
                Provider::Claude => version.split_whitespace().next() == Some("2.1.295"),
                Provider::Codex => version.trim() == "codex-cli 0.162.0",
                Provider::Gemini | Provider::OpenCode | Provider::Aider => true,
            } =>
        {
            None
        }
        Ok(version) => Some(format!(
            "Untested {} version {version}; run the CLI contract probes",
            provider.label()
        )),
        Err(error) => Some(format!(
            "Cannot verify {} version: {error}; run the CLI contract probes",
            provider.label()
        )),
    }
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_arguments_preserve_quotes_and_never_expand_shell_expressions() {
        let args = parse_launch_args(
            "--approve-for-me --model 'value with spaces' \"$(touch marker)\" ''",
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "--approve-for-me",
                "--model",
                "value with spaces",
                "$(touch marker)",
                ""
            ]
        );
        assert_eq!(parse_launch_args(&format_launch_args(&args)).unwrap(), args);
        assert!(parse_launch_args("--model 'unfinished").is_err());
        assert!(parse_launch_args("--model value\\").is_err());
        assert!(parse_launch_args("bad\0argument").is_err());
        assert!(parse_launch_args(&"a".repeat(16_385)).is_err());
        let mut record = record();
        record.launch_args = args.clone();
        assert_eq!(launch_args(&record).unwrap(), args);
        record.role = Role::Side;
        assert!(launch_args(&record).unwrap().is_empty());
    }
    #[test]
    fn interactive_agents_launch_without_overriding_settings_and_reject_session_profiles() {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!(
            "devdesk-interactive-agents-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = folder.join("devdesk.sock");
        let environment = ShellEnvironment {
            shell: "/bin/sh".into(),
            variables: [
                ("PATH".into(), folder.to_string_lossy().into_owned()),
                ("USER_SETTING".into(), "preserved".into()),
                (
                    "GEMINI_CLI_SYSTEM_DEFAULTS_PATH".into(),
                    folder
                        .join("missing-defaults.json")
                        .to_string_lossy()
                        .into_owned(),
                ),
            ]
            .into(),
            warning: None,
        };
        for provider in [Provider::Gemini, Provider::OpenCode, Provider::Aider] {
            let cli = folder.join(provider.command());
            std::fs::write(&cli, "#!/bin/sh\nexit 99\n").unwrap();
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut record = record();
            record.provider = provider;
            record.session_id = None;
            let build = |record: &Record, request: &Request| {
                InteractiveProvider(provider).build_launch(
                    &Context {
                        record,
                        request,
                        launch: 1,
                        token: "token",
                        socket: &socket,
                        helper: Path::new("/tmp/helper"),
                        lines: 5000,
                    },
                    &environment,
                )
            };
            let spec = build(&record, &Request::New).unwrap();
            assert_eq!(spec.program, cli.to_string_lossy());
            assert!(spec.args.is_empty());
            record.launch_args = vec!["--custom-flag".into(), "value with spaces".into()];
            assert_eq!(
                build(&record, &Request::New).unwrap().args,
                record.launch_args
            );
            assert_eq!(spec.cwd, record.cwd);
            if provider == Provider::Gemini {
                let settings: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(&spec.env["GEMINI_CLI_SYSTEM_DEFAULTS_PATH"]).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    settings["mcpServers"]["tervy_local"]["command"],
                    "/tmp/helper"
                );
                assert_eq!(
                    settings["mcpServers"]["tervy_local"]["env"]["DEVDESK_HOOK_TOKEN"],
                    "$DEVDESK_HOOK_TOKEN"
                );
                assert_eq!(spec.env["DEVDESK_MCP_ROLE"], "write");
                let defaults = folder.join("missing-defaults.json");
                std::fs::write(
                    &defaults,
                    r#"{"theme":"preserved","mcpServers":{"other":{"command":"other"}}}"#,
                )
                .unwrap();
                let merged = build(&record, &Request::New).unwrap();
                let settings: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(&merged.env["GEMINI_CLI_SYSTEM_DEFAULTS_PATH"]).unwrap(),
                )
                .unwrap();
                assert_eq!(settings["theme"], "preserved");
                assert_eq!(settings["mcpServers"]["other"]["command"], "other");
                std::fs::write(
                    &defaults,
                    r#"{"mcpServers":{"tervy_local":{"command":"other"}}}"#,
                )
                .unwrap();
                assert!(build(&record, &Request::New).is_err());
                std::fs::remove_file(defaults).unwrap();
            } else if provider == Provider::OpenCode {
                assert_eq!(spec.env["USER_SETTING"], "preserved");
                let config: serde_json::Value =
                    serde_json::from_str(&spec.env["OPENCODE_CONFIG_CONTENT"]).unwrap();
                assert_eq!(
                    config["mcp"]["tervy_local"]["command"],
                    serde_json::json!(["/tmp/helper", "mcp"])
                );
                assert_eq!(spec.env["DEVDESK_MCP_ROLE"], "write");
            } else {
                assert_eq!(spec.env, environment.variables);
            }
            assert_eq!(spec.scrollback_lines, 5000);
            assert_eq!(Provider::from_command(provider.command()), Some(provider));
            assert_eq!(
                serde_json::from_str::<Record>(&serde_json::to_string(&record).unwrap())
                    .unwrap()
                    .provider,
                provider
            );
            for request in [
                Request::Resume,
                Request::ResumePicker,
                Request::Prompt("prompt".into()),
                Request::Fork {
                    parent_session_id: "parent".into(),
                    question: "question".into(),
                },
            ] {
                assert!(build(&record, &request).is_err());
            }
            record.role = Role::Side;
            assert!(build(&record, &Request::New).is_err());
            record.role = Role::Primary;
            let mut runtime = Runtime::new(record, Request::New);
            runtime.info.status = Status::Running;
            runtime.apply(&event("PermissionRequest", None));
            runtime.input(true, std::time::Duration::ZERO);
            runtime.bell();
            assert_eq!(runtime.info.status, Status::Running);
        }
        std::fs::remove_dir_all(folder).unwrap();
    }
    fn record() -> Record {
        Record {
            id: "10000000-0000-4000-8000-000000000001".into(),
            project: "p".into(),
            name: "Claude notes".into(),
            cwd: PathBuf::from("/tmp/stored-project"),
            role: Role::Primary,
            launch_args: vec![],
            provider: crate::agent::Provider::Claude,
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
                Request::Fork { .. } | Request::Prompt(_) => unreachable!(),
            };
            expected.push("--mcp-config");
            assert_eq!(spec.args[..spec.args.len() - 3], expected);
            let mcp: serde_json::Value =
                serde_json::from_str(&spec.args[spec.args.len() - 3]).unwrap();
            assert_eq!(
                mcp["mcpServers"]["tervy_local"]["args"],
                serde_json::json!(["mcp"])
            );
            assert_eq!(spec.args[spec.args.len() - 2], "--settings");
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
        let prompt = Request::Prompt("Make the font larger\nWrite proposal.toml".into());
        let context = Context {
            record: &record,
            request: &prompt,
            launch: 8,
            token: "test-token",
            socket: Path::new("/tmp/devdesk.sock"),
            helper: Path::new("/tmp/devdesk"),
            lines: 5000,
        };
        let spec = ClaudeProvider.build_launch(&context, &environment).unwrap();
        assert_eq!(
            spec.args.last().unwrap(),
            "Make the font larger\nWrite proposal.toml"
        );
        let mut side = record.clone();
        side.role = Role::Side;
        assert!(
            ClaudeProvider
                .build_launch(
                    &Context {
                        record: &side,
                        ..context
                    },
                    &environment
                )
                .is_err()
        );
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
            &spec.args[6..],
            [
                "--restricted",
                "--strict-mcp-config",
                "--tools=Read,Grep,Glob,mcp__tervy_local__list_prompts,mcp__tervy_local__read_prompt",
                "--disallowedTools=Edit,Write,NotebookEdit,Bash,Agent,WebFetch,WebSearch,ShareOnboardingGuide",
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
        ];
        expected.extend(spec.args[2..].iter().cloned());
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
        old.as_object_mut().unwrap().remove("provider");
        old.as_object_mut().unwrap().remove("parent");
        old.as_object_mut().unwrap().remove("snapshot_at");
        old.as_object_mut().unwrap().remove("launch_args");
        let old: Record = serde_json::from_value(old).unwrap();
        assert!(old.parent.is_none() && old.snapshot_at.is_none());
        assert_eq!(old.provider, Provider::Claude);
        assert!(old.launch_args.is_empty());
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn codex_inference_menus_cancel_failed_turn_bell_and_decay() {
        use std::time::Duration;
        let mut record = record();
        record.provider = Provider::Codex;
        let mut runtime = Runtime::new(record, Request::New);
        runtime.info.status = Status::Idle;
        // Enter can open a menu: activity is explicitly inferred, never Working.
        runtime.input(true, Duration::ZERO);
        assert_eq!(runtime.info.status, Status::ActiveInferred);
        assert_eq!(runtime.decay_at, Some(Duration::from_secs(600)));
        // Esc/cancel cannot prove completion. No-output expiry handles it.
        runtime.input(false, Duration::from_secs(1));
        assert_eq!(runtime.info.status, Status::ActiveInferred);
        runtime.expire_inferred();
        assert_eq!(runtime.info.status, Status::Idle);
        // Any command's BEL is Attention, never proof of an approval prompt.
        runtime.input(true, Duration::from_secs(10));
        runtime.bell();
        runtime.bell();
        assert_eq!(runtime.info.status, Status::Attention);
        runtime.input(false, Duration::from_secs(11));
        assert_eq!(runtime.info.status, Status::ActiveInferred);
        runtime.bell();
        runtime.expire_inferred();
        assert_eq!(runtime.info.status, Status::Attention);
        runtime.input(false, Duration::from_secs(611));
        assert_eq!(runtime.info.status, Status::Idle);
        // A failed turn without notify stays inferred until decay or PTY exit.
        runtime.input(true, Duration::from_secs(612));
        runtime.apply(&event("StopFailure", None));
        assert_eq!(runtime.info.status, Status::ActiveInferred);
        runtime.observe_title("20000000-0000-4000-8000-0000000...");
        runtime.apply(&HookEvent {
            session_id: Some("30000000-0000-4000-8000-000000000003".into()),
            ..event("agent-turn-complete", None)
        });
        assert_eq!(runtime.info.status, Status::ActiveInferred);
        runtime.apply(&HookEvent {
            session_id: Some("20000000-0000-4000-8000-000000000002".into()),
            ..event("agent-turn-complete", None)
        });
        assert_eq!(runtime.info.status, Status::Idle);
        assert!(runtime.decay_at.is_none());
        runtime.exited();
        runtime.bell();
        runtime.input(true, Duration::from_secs(613));
        runtime.apply(&event("agent-turn-complete", None));
        assert_eq!(runtime.info.status, Status::Exited);
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
    #[test]
    fn codex_notify_before_title_is_correlated_without_accepting_internal_threads() {
        let mut record = record();
        record.provider = Provider::Codex;
        record.session_id = None;
        let mut runtime = Runtime::new(record, Request::New);
        let visible = "20000000-0000-4000-8000-000000000002";
        let internal = "30000000-0000-4000-8000-000000000003";
        for id in [visible, internal] {
            runtime.apply(&HookEvent {
                session_id: Some(id.into()),
                ..event("agent-turn-complete", None)
            });
        }
        assert!(runtime.info.record.session_id.is_none());
        assert!(!runtime.observe_title("invalid title"));
        assert!(runtime.observe_title("20000000-0000-4000-8000-0000000..."));
        assert_eq!(runtime.info.record.session_id.as_deref(), Some(visible));
        assert_eq!(runtime.info.status, Status::Idle);
        runtime.apply(&HookEvent {
            session_id: Some(internal.into()),
            ..event("agent-turn-complete", None)
        });
        assert_eq!(runtime.info.record.session_id.as_deref(), Some(visible));
        runtime.launching();
        assert!(runtime.codex_pending_ids.is_empty());
    }

    #[test]
    fn codex_pending_ids_are_bounded_and_parent_identity_is_never_accepted() {
        let mut record = record();
        record.provider = Provider::Codex;
        record.session_id = None;
        let parent = "20000000-0000-4000-8000-000000000002";
        let mut runtime = Runtime::new(
            record,
            Request::Fork {
                parent_session_id: parent.into(),
                question: "synthetic".into(),
            },
        );
        runtime.apply(&HookEvent {
            session_id: Some(parent.into()),
            ..event("agent-turn-complete", None)
        });
        assert!(runtime.codex_pending_ids.is_empty());
        for _ in 0..20 {
            runtime.apply(&HookEvent {
                session_id: Some(uuid::Uuid::new_v4().to_string()),
                ..event("agent-turn-complete", None)
            });
        }
        assert_eq!(runtime.codex_pending_ids.len(), 8);
        runtime.exited();
        assert!(!runtime.observe_title("20000000-0000-4000-8000-0000000..."));
        assert!(runtime.info.record.session_id.is_none());
    }
}
