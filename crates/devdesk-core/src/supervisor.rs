//! One actor owns every PTY. Views hold only cheap terminal handles and send
//! commands; no view destructor kills a process. Lifecycle callbacks use the
//! actor mailbox, while output/wakeups go directly to the terminal view.
use crate::config::{self, Config};
use crate::env::ShellEnvironment;
use crate::{
    agent::{self, AgentProvider},
    ipc, layout, mcp, meetings, prompts,
};
use crate::{
    process,
    store::{Orphan, Store},
};
use devdesk_term::{
    Dimensions, SpawnSpec, TerminalEvents, TerminalHandle, TerminalSession, TerminalSignal,
};
use notify::Watcher;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchId(pub u64);
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Exited { code: Option<i32> },
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub agent: Option<agent::Info>,
    pub process: Option<String>,
    pub config_changed: bool,
    pub pid: Option<u32>,
    pub id: SessionId,
    pub launch: LaunchId,
    pub project: String,
    pub name: String,
    pub status: SessionStatus,
}

pub enum Command {
    ListMeetings,
    StartMeeting {
        notes_delivery: meetings::NotesDelivery,
    },
    StopMeeting,
    RetryMeeting(String),
    OpenMeetingAudio {
        id: String,
        microphone: bool,
    },
    SaveMeetingNotes {
        id: String,
        notes: String,
    },
    RenameMeeting {
        id: String,
        title: String,
    },
    DeleteMeetingAudio(String),
    DeleteMeeting(String),
    ListPrompts {
        query: String,
        limit: usize,
        offset: usize,
    },
    ReadPrompt(String),
    WritePrompt(prompts::Write),
    DeletePrompt {
        id: String,
        revision: i64,
    },
    ResolvePromptDelete {
        request_id: u64,
        approved: bool,
    },
    NewAgent {
        project: String,
        picker: bool,
        provider: agent::Provider,
    },
    NewAgentWithArgs {
        project: String,
        picker: bool,
        provider: agent::Provider,
        args: Vec<String>,
    },
    Customize {
        project: String,
        provider: agent::Provider,
        request: String,
    },
    ReviewCustomization,
    ApplyCustomization(String),
    UndoCustomization,
    ResumeAgent(SessionId),
    ResumeAgentWithArgs(SessionId, Vec<String>),
    ForkAgent {
        parent: SessionId,
        question: String,
    },
    AgentInput {
        id: SessionId,
        launch: LaunchId,
        enter: bool,
    },
    AgentBell {
        id: SessionId,
        launch: LaunchId,
    },
    AgentTitle {
        id: SessionId,
        launch: LaunchId,
        title: String,
    },
    AgentExitSummary {
        id: SessionId,
        launch: LaunchId,
        session_id: String,
    },
    RemoveSession(SessionId),
    RemoveProject(String),
    SaveLayout(layout::Layout),
    Restore,
    NewTerminalPreset {
        project: String,
        name: String,
    },
    NewShell {
        project: String,
        size: Dimensions,
    },
    Close {
        id: SessionId,
        confirmed: Option<LaunchId>,
    },
    Restart(SessionId),
    Start(SessionId),
    Stop(SessionId),
    StartAll(String),
    StopAll(String),
    ProjectOpened(String),
    ShowOrphans,
    ResolveOrphans {
        kill: bool,
    },
    Rename {
        id: SessionId,
        name: String,
    },
    AddProject(PathBuf),
    AddProcess {
        project: String,
        name: String,
        command: String,
    },
    EditProcess {
        project: String,
        process: String,
        name: String,
        command: String,
        autostart: bool,
    },
    ReloadConfig,
    Shutdown,
}
pub enum CoreEvent {
    MeetingsListed(Vec<meetings::Meeting>),
    MeetingUpdated(meetings::Meeting),
    MeetingDeleted(String),
    MeetingError(String),
    PromptsListed {
        query: String,
        offset: usize,
        items: Vec<prompts::Summary>,
    },
    PromptRead {
        id: String,
        prompt: Option<prompts::Prompt>,
    },
    PromptSaved {
        prompt: prompts::Prompt,
        from_ui: bool,
    },
    PromptDeleted(String),
    PromptDeleteApproval {
        request_id: u64,
        title: String,
        agent: String,
    },
    PromptError(String),
    /// Approximate physical footprint of the session process tree, in bytes.
    /// Launch identity prevents delayed samples from attaching to a restart.
    MemoryUsage(Vec<(SessionId, LaunchId, Option<u64>)>),
    Restored {
        saved: bool,
        layout: layout::Layout,
        keys: BTreeMap<String, SessionId>,
    },
    Orphans(Vec<Orphan>),
    CustomizationPreview(crate::customization::Preview),
    CustomizationApplied {
        undone: bool,
    },
    Config(Config),
    ConfigError(String),
    ProjectAdded(String),
    ProcessAdded(String),
    ProcessEdited {
        project: String,
        process: String,
    },
    ProcessAddFailed {
        project: String,
        error: String,
    },
    ProjectRemoved(String),
    SessionChanged(SessionInfo),
    SessionStarted {
        info: SessionInfo,
        handle: TerminalHandle,
        events: TerminalEvents,
        warning: Option<String>,
        spawn_started: Option<Instant>,
    },
    SessionClosed(SessionId),
    ConfirmClose {
        id: SessionId,
        launch: LaunchId,
        job: String,
    },
    ShutdownProgress {
        remaining: usize,
    },
    ShutdownComplete {
        clean: bool,
    },
}

#[derive(Clone)]
pub struct SupervisorHandle {
    sender: mpsc::Sender<Message>,
}
impl SupervisorHandle {
    pub fn send(&self, command: Command) {
        let _ = self.sender.send(Message::Command(command));
    }
}
pub struct Supervisor {
    pub handle: SupervisorHandle,
    pub events: async_channel::Receiver<CoreEvent>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Supervisor {
    pub fn start(path: PathBuf, probe: Option<String>) -> std::io::Result<Self> {
        Self::start_with_helper(path, probe, std::env::current_exe()?)
    }
    /// Explicit helper location supports bundled binaries and headless contract
    /// tests. Production uses its own executable; no helper path enters TOML.
    pub fn start_with_helper(
        path: PathBuf,
        probe: Option<String>,
        helper: PathBuf,
    ) -> std::io::Result<Self> {
        if !helper.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Hook helper must be an absolute path",
            ));
        }
        let (sender, receiver) = mpsc::channel();
        let (events, event_rx) = async_channel::unbounded();
        let handle = SupervisorHandle {
            sender: sender.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("devdesk-supervisor".into())
            .spawn(move || {
                let mut actor = Actor::new(path, probe, sender, events);
                actor.helper = Some(helper);
                actor.run(receiver);
            })?;
        Ok(Self {
            handle,
            events: event_rx,
            thread: Some(thread),
        })
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        self.handle.send(Command::Shutdown);
        // Application runtime has already returned; never called in a view.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Message {
    Meeting {
        id: String,
        update: meetings::Update,
    },
    Mcp(mcp::Request),
    MemorySampled(Vec<(SessionId, LaunchId, Option<u64>)>),
    HooksReady,
    Reaped {
        id: SessionId,
        launch: LaunchId,
        result: Result<TerminalHandle, String>,
    },
    Command(Command),
    ConfigChanged,
    WatchError(String),
    Spawned {
        id: SessionId,
        launch: LaunchId,
        result: Box<Result<(TerminalSession, TerminalEvents), String>>,
        warning: Option<String>,
        spawn_started: Option<Instant>,
    },
    Lifecycle {
        id: SessionId,
        launch: LaunchId,
        signal: TerminalSignal,
    },
}

pub trait Clock: Send {
    fn now(&self) -> Duration;
}
struct SystemClock(Instant);
impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

#[derive(Clone, Debug)]
struct StopPlan {
    started: Duration,
    next_check: Duration,
    stage: u8,
    first_signal: i32,
    term_after: Duration,
    kill_after: Duration,
}
impl StopPlan {
    fn new(now: Duration) -> Self {
        Self {
            started: now,
            next_check: now,
            stage: 0,
            first_signal: libc::SIGHUP,
            term_after: Duration::from_secs(2),
            kill_after: Duration::from_secs(5),
        }
    }
    fn for_process(now: Duration, spec: &config::ProcessSpec) -> Self {
        let mut plan = Self::new(now);
        plan.first_signal = match spec.stop_signal.as_str() {
            "SIGTERM" => libc::SIGTERM,
            "SIGHUP" => libc::SIGHUP,
            _ => libc::SIGINT,
        };
        plan.term_after = Duration::from_secs(spec.stop_timeout_secs);
        plan.kill_after = plan.term_after + Duration::from_secs(3);
        plan
    }
    fn for_agent(now: Duration) -> Self {
        let mut stop = Self::new(now);
        stop.first_signal = libc::SIGTERM;
        stop.term_after = Duration::from_secs(3);
        stop.kill_after = Duration::from_secs(3);
        stop
    }
    fn due_signal(&mut self, now: Duration) -> Option<i32> {
        let age = now.saturating_sub(self.started);
        let next = if age >= self.kill_after {
            3
        } else if age >= self.term_after {
            2
        } else {
            1
        };
        if next > self.stage {
            self.stage = next;
            Some(match next {
                1 => self.first_signal,
                2 => libc::SIGTERM,
                _ => libc::SIGKILL,
            })
        } else if self.stage == 3 {
            Some(libc::SIGKILL)
        } else {
            None
        }
    }
}
struct Entry {
    agent: Option<agent::Runtime>,
    cwd: Option<PathBuf>,
    process: Option<process::Runtime>,
    desired: Option<config::ProcessSpec>,
    history: Option<TerminalHandle>,
    joining: bool,
    info: SessionInfo,
    session: Option<TerminalSession>,
    size: Dimensions,
    stopping: Option<StopPlan>,
    close: bool,
    restart: bool,
    drained: bool,
    exited: bool,
}
struct Actor {
    meeting_stop: Option<(String, mpsc::Sender<()>)>,
    mcp: Option<mcp::Server>,
    pending_prompt_deletes: BTreeMap<u64, (mcp::Request, Duration)>,
    next_prompt_delete: u64,
    memory_due: Option<Duration>,
    memory_sampling: bool,
    // Removal is permanent metadata deletion; close alone only dismisses a tab.
    helper: Option<PathBuf>,
    removing: BTreeSet<SessionId>,
    hooks: Option<ipc::Server>,
    layout_due: Option<Duration>,
    pending_layout: Option<layout::Layout>,
    restored: bool,
    store: Option<Store>,
    orphans: Vec<Orphan>,
    recovery_pending: bool,
    orphan_kill_at: Option<Duration>,
    orphan_check_at: Option<Duration>,
    opened: BTreeSet<String>,
    path: PathBuf,
    probe: Option<String>,
    config: Config,
    entries: BTreeMap<SessionId, Entry>,
    sender: mpsc::Sender<Message>,
    events: async_channel::Sender<CoreEvent>,
    clock: Box<dyn Clock>,
    next_id: u64,
    next_launch: u64,
    config_due: Option<Duration>,
    codex_scan_at: Option<Duration>,
    shutdown_deadline: Option<Duration>,
    watcher: Option<notify::RecommendedWatcher>,
}
impl Actor {
    fn new(
        path: PathBuf,
        probe: Option<String>,
        sender: mpsc::Sender<Message>,
        events: async_channel::Sender<CoreEvent>,
    ) -> Self {
        Self {
            meeting_stop: None,
            mcp: None,
            pending_prompt_deletes: BTreeMap::new(),
            next_prompt_delete: 1,
            memory_due: None,
            memory_sampling: false,
            helper: None,
            removing: BTreeSet::new(),
            hooks: None,
            layout_due: None,
            pending_layout: None,
            restored: false,
            store: None,
            orphans: vec![],
            recovery_pending: false,
            orphan_kill_at: None,
            orphan_check_at: None,
            opened: BTreeSet::new(),
            path,
            probe,
            config: Config::default(),
            entries: BTreeMap::new(),
            sender,
            events,
            clock: Box::new(SystemClock(Instant::now())),
            next_id: 1,
            next_launch: 1,
            config_due: None,
            codex_scan_at: None,
            shutdown_deadline: None,
            watcher: None,
        }
    }
    fn emit(&self, event: CoreEvent) {
        let _ = self.events.try_send(event);
    }
    fn changed(&self, id: SessionId) {
        if let Some(entry) = self.entries.get(&id) {
            let mut info = entry.info.clone();
            info.agent = entry.agent.as_ref().map(|runtime| runtime.info.clone());
            if let Some(process) = &entry.process {
                info.status = process.state.clone();
            }
            self.emit(CoreEvent::SessionChanged(info));
        }
    }
    fn load_config(&mut self) {
        match config::initialize(&self.path).and_then(|_| config::read(&self.path)) {
            Ok(config) => {
                self.config = config;
                self.emit(CoreEvent::Config(self.config.clone()));
                self.sync_processes();
            }
            Err(error) => self.emit(CoreEvent::ConfigError(error)),
        }
    }
    fn run(mut self, receiver: mpsc::Receiver<Message>) {
        if self.probe.is_some() {
            self.config.projects.push(config::Project {
                id: "probe".into(),
                name: "DevDesk".into(),
                path: std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()),
                processes: vec![],
                terminals: vec![],
                agents: config::AgentDefaults::default(),
            });
            if self.probe.as_deref() == Some("many") {
                for id in ["probe2", "probe3"] {
                    let mut project = self.config.projects[0].clone();
                    project.id = id.into();
                    project.name = id.into();
                    self.config.projects.push(project);
                }
            }
            self.emit(CoreEvent::Config(self.config.clone()));
        } else {
            if let Err(error) = config::initialize(&self.path) {
                self.emit(CoreEvent::ConfigError(error));
            }
            match Store::open(&self.path.with_file_name("state.db")) {
                Ok(store) => {
                    match store.survivors() {
                        Ok(orphans) => {
                            self.recovery_pending = !orphans.is_empty();
                            self.orphans = orphans;
                        }
                        Err(error) => {
                            self.emit(CoreEvent::ConfigError(error));
                            self.recovery_pending = true;
                        }
                    }
                    self.store = Some(store);
                    self.list_prompts(String::new(), 100, 0);
                    self.recover_meetings();
                }
                Err(error) => self.emit(CoreEvent::ConfigError(format!(
                    "Session ledger unavailable: {error}"
                ))),
            }
            self.load_config();
            let sender = self.sender.clone();
            match ipc::Server::bind(
                self.path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(".")),
                move || {
                    let _ = sender.send(Message::HooksReady);
                },
            ) {
                Ok(server) => self.hooks = Some(server),
                Err(error) => self.emit(CoreEvent::ConfigError(format!(
                    "Hook server unavailable: {error}"
                ))),
            }
            if let Some(hook) = &self.hooks {
                let sender = self.sender.clone();
                match mcp::Server::bind(hook.path.with_file_name("mcp.sock"), move |request| {
                    let _ = sender.send(Message::Mcp(request));
                }) {
                    Ok(server) => self.mcp = Some(server),
                    Err(error) => self.emit(CoreEvent::ConfigError(format!(
                        "MCP bridge unavailable: {error}"
                    ))),
                }
            }
            if self.recovery_pending && !self.orphans.is_empty() {
                self.emit(CoreEvent::Orphans(self.orphans.clone()));
            }
            // FSEvents reports canonical paths (e.g. /private/var, not /var).
            if let Ok(path) = self.path.canonicalize() {
                self.path = path;
            }
            let sender = self.sender.clone();
            let path = self.path.clone();
            match notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                match result {
                    Ok(event) if event.paths.contains(&path) => {
                        let _ = sender.send(Message::ConfigChanged);
                    }
                    Err(error) => {
                        let _ = sender.send(Message::WatchError(error.to_string()));
                    }
                    _ => {}
                }
            }) {
                Ok(mut watcher) => {
                    if let Some(parent) = self.path.parent() {
                        match watcher.watch(parent, notify::RecursiveMode::NonRecursive) {
                            Ok(()) => self.watcher = Some(watcher),
                            Err(error) => self.emit(CoreEvent::ConfigError(format!(
                                "Cannot watch config: {error}"
                            ))),
                        }
                    }
                }
                Err(error) => self.emit(CoreEvent::ConfigError(format!(
                    "Cannot watch config: {error}"
                ))),
            }
        }
        loop {
            let now = self.clock.now();
            let next = self
                .entries
                .values()
                .filter_map(|entry| entry.stopping.as_ref().map(|stop| stop.next_check))
                .chain(
                    self.entries
                        .values()
                        .filter_map(|entry| {
                            entry.process.as_ref().and_then(|process| process.retry_at)
                        })
                        .filter(|_| {
                            !self.recovery_pending
                                && self.orphan_check_at.is_none()
                                && self.shutdown_deadline.is_none()
                        }),
                )
                .chain(
                    self.entries
                        .values()
                        .filter_map(|entry| entry.agent.as_ref().and_then(|agent| agent.decay_at)),
                )
                .chain(self.codex_scan_at)
                .chain(self.memory_due)
                .chain(self.orphan_check_at)
                .chain(self.config_due)
                .chain(self.layout_due)
                .chain(self.pending_prompt_deletes.values().map(|(_, due)| *due))
                .chain(self.shutdown_deadline)
                .min();
            let message = match next {
                Some(deadline) => match receiver.recv_timeout(deadline.saturating_sub(now)) {
                    Ok(message) => Some(message),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(_) => break,
                },
                None => match receiver.recv() {
                    Ok(message) => Some(message),
                    Err(_) => break,
                },
            };
            if let Some(message) = message {
                self.message(message);
            }
            self.timers();
            self.sample_memory();
            if self.shutdown_deadline.is_some()
                && self.entries.is_empty()
                && self.orphan_check_at.is_none()
                && self.meeting_stop.is_none()
            {
                self.flush_layout();
                self.emit(CoreEvent::ShutdownComplete { clean: true });
                break;
            }
            if self
                .shutdown_deadline
                .is_some_and(|deadline| self.clock.now() >= deadline)
            {
                for entry in self.entries.values() {
                    if let Some(session) = &entry.session {
                        let _ = session.signal(libc::SIGKILL);
                    }
                }
                self.emit(CoreEvent::ShutdownComplete { clean: false });
                break;
            }
        }
    }
    fn message(&mut self, message: Message) {
        match message {
            Message::Meeting { id, update } => self.meeting_update(id, update),
            Message::Mcp(request) => self.mcp_request(request),
            Message::MemorySampled(samples) => {
                self.memory_sampling = false;
                self.memory_due = Some(self.clock.now() + Duration::from_secs(5));
                let samples = samples
                    .into_iter()
                    .filter(|(id, launch, _)| {
                        self.entries.get(id).is_some_and(|entry| {
                            entry.info.launch == *launch && entry.session.is_some() && !entry.exited
                        })
                    })
                    .collect();
                self.emit(CoreEvent::MemoryUsage(samples));
            }
            Message::HooksReady => self.hook_messages(),
            Message::Reaped { id, launch, result } => {
                if !self
                    .entries
                    .get(&id)
                    .is_some_and(|entry| entry.info.launch == launch)
                {
                    return;
                }
                match result {
                    Ok(history) => {
                        let entry = self.entries.get_mut(&id).expect("registered handoff");
                        entry.joining = false;
                        entry.history = Some(history);
                        if let Some(process) = &mut entry.process {
                            process.cleaned(self.clock.now());
                        }
                        self.complete_stop(id);
                    }
                    Err(error) => {
                        let entry = self.entries.get_mut(&id).expect("registered handoff");
                        entry.joining = false;
                        entry.info.status = SessionStatus::Failed(error.clone());
                        if let Some(runtime) = &mut entry.agent {
                            runtime.exited();
                        }
                        if let Some(process) = &mut entry.process {
                            process.fail(error);
                            entry.restart = false;
                        }
                        self.complete_stop(id);
                    }
                }
            }
            Message::ConfigChanged => {
                self.config_due = Some(self.clock.now() + Duration::from_millis(120))
            }
            Message::WatchError(error) => {
                self.emit(CoreEvent::ConfigError(format!("Config watcher: {error}")))
            }
            Message::Command(command) => self.command(command),
            Message::Spawned {
                id,
                launch,
                result,
                warning,
                spawn_started,
            } => {
                if !self
                    .entries
                    .get(&id)
                    .is_some_and(|entry| entry.info.launch == launch)
                {
                    return;
                }
                match *result {
                    Ok((session, events)) => {
                        let owner = self.owner(id);
                        let recorded = if self.probe.is_some() {
                            Ok(())
                        } else {
                            self.store
                                .as_ref()
                                .ok_or_else(|| {
                                    "Session ledger unavailable; refusing untracked spawn"
                                        .to_string()
                                })
                                .and_then(|store| store.insert(session.owned, launch.0, &owner))
                        };
                        if let Err(error) = recorded {
                            let entry = self.entries.get_mut(&id).expect("registered spawn");
                            entry.close = true;
                            entry.session = Some(session);
                            entry.stopping = Some(StopPlan::new(self.clock.now()));
                            self.emit(CoreEvent::ConfigError(error));
                            return;
                        }
                        let handle = session.handle.clone();
                        let entry = self.entries.get_mut(&id).expect("checked session");
                        entry.info.pid = Some(session.pid);
                        entry.session = Some(session);
                        if entry.stopping.is_none() && !entry.exited {
                            entry.info.status = SessionStatus::Running;
                        }
                        if let Some(process) = &mut entry.process {
                            process.spawned(self.clock.now());
                            entry.info.status = process.state.clone();
                        }
                        if let Some(runtime) = &mut entry.agent
                            && runtime.info.record.provider == agent::Provider::Codex
                        {
                            handle.track_output_activity();
                            if runtime.info.status == agent::Status::Launching {
                                runtime.info.status = agent::Status::Idle;
                                if matches!(runtime.request, agent::Request::Fork { .. }) {
                                    runtime.input(true, self.clock.now());
                                }
                            }
                        }
                        if let Some(runtime) = &mut entry.agent
                            && !runtime.info.record.provider.tracks_sessions()
                        {
                            runtime.info.status = agent::Status::Running;
                        }
                        entry.info.agent = entry.agent.as_ref().map(|runtime| runtime.info.clone());
                        self.arm_codex_scan();
                        self.emit(CoreEvent::SessionStarted {
                            info: self.entries[&id].info.clone(),
                            handle,
                            events,
                            warning,
                            spawn_started,
                        });
                    }
                    Err(error) => {
                        let entry = self.entries.get_mut(&id).expect("checked session");
                        entry.exited = true;
                        entry.drained = true;
                        entry.info.status = SessionStatus::Failed(error.clone());
                        if let Some(runtime) = &mut entry.agent {
                            runtime.exited();
                        }
                        if let Some(process) = &mut entry.process {
                            process.spawn_failed(error, self.clock.now());
                            entry.info.status = process.state.clone();
                        }
                        if entry.close {
                            self.removing.remove(&id);
                            self.entries.remove(&id);
                            self.emit(CoreEvent::SessionClosed(id));
                        } else {
                            self.changed(id);
                        }
                    }
                }
            }
            Message::Lifecycle { id, launch, signal } => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    if entry.info.launch != launch {
                        return;
                    }
                    match signal {
                        TerminalSignal::ChildExit(status) => {
                            entry.exited = true;
                            if let Some(runtime) = &mut entry.agent {
                                runtime.exited();
                            }
                            if let Some(process) = &mut entry.process {
                                process.exited(status.code());
                            }
                            if entry.stopping.is_none() {
                                entry.info.status = SessionStatus::Exited {
                                    code: status.code(),
                                };
                            }
                            // A dead shell may leave jobs holding its PTY. Clean
                            // those up even if the terminal never reaches EOF.
                            entry.stopping.get_or_insert_with(|| {
                                entry.process.as_ref().map_or_else(
                                    || {
                                        if entry.agent.is_some() {
                                            StopPlan::for_agent(self.clock.now())
                                        } else {
                                            StopPlan::new(self.clock.now())
                                        }
                                    },
                                    |process| {
                                        StopPlan::for_process(self.clock.now(), &process.spec)
                                    },
                                )
                            });
                        }
                        TerminalSignal::Exit => entry.drained = true,
                        TerminalSignal::Wakeup => {
                            unreachable!("output never enters the supervisor")
                        }
                    }
                    self.changed(id);
                }
            }
        }
    }
    /// The actor copies ownership identities; all process enumeration and
    /// memory syscalls run on one worker. Empty workspaces have no sampler
    /// timer, and at most one batch can be in flight during a slow OS query.
    fn sample_memory(&mut self) {
        if self.shutdown_deadline.is_some()
            || !self
                .entries
                .values()
                .any(|entry| entry.session.is_some() && !entry.exited)
        {
            self.memory_due = None;
            return;
        }
        if self.memory_sampling {
            return;
        }
        let now = self.clock.now();
        if self.memory_due.is_some_and(|due| due > now) {
            return;
        }
        let roots: Vec<_> = self
            .entries
            .values()
            .filter_map(|entry| {
                if entry.exited {
                    return None;
                }
                let session = entry.session.as_ref()?;
                Some((entry.info.id, entry.info.launch, session.owned_sessions()))
            })
            .collect();
        self.memory_due = None;
        self.memory_sampling = true;
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let groups: Vec<_> = roots
                .iter()
                .map(|(_, _, sessions)| sessions.clone())
                .collect();
            let totals = devdesk_term::kill::memory_footprints(&groups);
            let samples = roots
                .into_iter()
                .zip(totals)
                .map(|((id, launch, _), bytes)| (id, launch, bytes))
                .collect();
            let _ = sender.send(Message::MemorySampled(samples));
        });
    }
    fn command(&mut self, command: Command) {
        if self.shutdown_deadline.is_some() && !matches!(command, Command::Shutdown) {
            return;
        }
        match command {
            Command::ListMeetings => self.list_meetings(),
            Command::StartMeeting { notes_delivery } => self.start_meeting(notes_delivery),
            Command::StopMeeting => self.stop_meeting(),
            Command::RetryMeeting(id) => self.retry_meeting(id),
            Command::OpenMeetingAudio { id, microphone } => self.open_meeting_audio(id, microphone),
            Command::SaveMeetingNotes { id, notes } => self.edit_meeting(id, |meeting| {
                if notes.len() > 512 * 1024 {
                    return Err("Notes exceed 512 KB".into());
                }
                meeting.notes = notes;
                Ok(())
            }),
            Command::RenameMeeting { id, title } => self.edit_meeting(id, |meeting| {
                let title = title.trim();
                if title.is_empty() || title.len() > 160 {
                    return Err("Meeting title must be 1–160 bytes".into());
                }
                meeting.title = title.into();
                Ok(())
            }),
            Command::DeleteMeetingAudio(id) => self.delete_meeting_audio(id),
            Command::DeleteMeeting(id) => self.delete_meeting(id),
            Command::ListPrompts {
                query,
                limit,
                offset,
            } => self.list_prompts(query, limit, offset),
            Command::ReadPrompt(id) => {
                match self
                    .store
                    .as_ref()
                    .ok_or("Prompt store unavailable".to_string())
                    .and_then(|store| store.read_prompt(&id))
                {
                    Ok(prompt) => self.emit(CoreEvent::PromptRead { id, prompt }),
                    Err(error) => self.emit(CoreEvent::PromptError(error)),
                }
            }
            Command::WritePrompt(write) => {
                match self
                    .store
                    .as_ref()
                    .ok_or("Prompt store unavailable".to_string())
                    .and_then(|store| store.write_prompt(&write))
                {
                    Ok(prompt) => {
                        self.emit(CoreEvent::PromptSaved {
                            prompt,
                            from_ui: true,
                        });
                        self.list_prompts(String::new(), 100, 0);
                    }
                    Err(error) => self.emit(CoreEvent::PromptError(error)),
                }
            }
            Command::DeletePrompt { id, revision } => {
                match self
                    .store
                    .as_ref()
                    .ok_or("Prompt store unavailable".to_string())
                    .and_then(|store| store.delete_prompt(&id, revision))
                {
                    Ok(()) => {
                        self.emit(CoreEvent::PromptDeleted(id));
                        self.list_prompts(String::new(), 100, 0);
                    }
                    Err(error) => self.emit(CoreEvent::PromptError(error)),
                }
            }
            Command::ResolvePromptDelete {
                request_id,
                approved,
            } => self.resolve_prompt_delete(request_id, approved),
            Command::NewAgent {
                project,
                picker,
                provider,
            } => self.new_agent(project, picker, provider, None, vec![]),
            Command::NewAgentWithArgs {
                project,
                picker,
                provider,
                args,
            } => self.new_agent(project, picker, provider, None, args),
            Command::Customize {
                project,
                provider,
                request,
            } => {
                self.new_agent(project, false, provider, Some(request), vec![]);
            }
            Command::ReviewCustomization => match crate::customization::Draft::review(&self.path) {
                Ok(proposal) => self.emit(CoreEvent::CustomizationPreview(proposal)),
                Err(error) => self.emit(CoreEvent::ConfigError(error)),
            },
            Command::ApplyCustomization(proposal) => {
                match crate::customization::Draft::apply(&self.path, &proposal) {
                    Ok(config) => {
                        self.config = config;
                        self.emit(CoreEvent::Config(self.config.clone()));
                        self.sync_processes();
                        self.emit(CoreEvent::CustomizationApplied { undone: false });
                    }
                    Err(error) => self.emit(CoreEvent::ConfigError(error)),
                }
            }
            Command::UndoCustomization => match crate::customization::Draft::undo(&self.path) {
                Ok(config) => {
                    self.config = config;
                    self.emit(CoreEvent::Config(self.config.clone()));
                    self.sync_processes();
                    self.emit(CoreEvent::CustomizationApplied { undone: true });
                }
                Err(error) => self.emit(CoreEvent::ConfigError(error)),
            },
            Command::AgentInput { id, launch, enter } => self.agent_input(id, launch, Some(enter)),
            Command::AgentBell { id, launch } => self.agent_input(id, launch, None),
            Command::AgentTitle { id, launch, title } => {
                let completed = self.entries.get_mut(&id).and_then(|entry| {
                    if entry.info.launch != launch || entry.exited || self.removing.contains(&id) {
                        return None;
                    }
                    let runtime = entry.agent.as_mut()?;
                    runtime
                        .observe_title(&title)
                        .then(|| runtime.info.record.clone())
                });
                if let Some(record) = completed {
                    if let Some(store) = &self.store
                        && let Err(error) = store.save_agent(&record)
                    {
                        self.emit(CoreEvent::ConfigError(error));
                    }
                    self.track_codex_descendants(id);
                    self.arm_codex_scan();
                    self.changed(id);
                }
            }
            Command::AgentExitSummary {
                id,
                launch,
                session_id,
            } => self.agent_exit_summary(id, launch, session_id),
            Command::ForkAgent { parent, question } => self.fork_agent(parent, question),
            Command::ResumeAgent(id) => self.resume_agent(id, None),
            Command::ResumeAgentWithArgs(id, args) => self.resume_agent(id, Some(args)),
            Command::RemoveSession(id) => self.remove_session(id),
            Command::RemoveProject(project) => self.remove_project(project),
            Command::SaveLayout(mut layout) => {
                for tab in &mut layout.tabs {
                    if matches!(tab.kind, layout::Kind::Shell)
                        && let Some(entry) = self
                            .entries
                            .values()
                            .find(|entry| self.key(entry) == tab.key)
                    {
                        tab.cwd = entry
                            .session
                            .as_ref()
                            .and_then(|session| devdesk_term::kill::cwd(session.owned.leader).ok())
                            .or_else(|| entry.cwd.clone());
                    }
                }
                self.pending_layout = Some(layout);
                self.layout_due = Some(self.clock.now() + Duration::from_millis(250));
            }
            Command::Restore => self.restore(),
            Command::Start(id) => self.start_process(id, true),
            Command::Stop(id) => self.stop(id, false, false),
            Command::StartAll(project) => {
                let ids: Vec<_> = self
                    .entries
                    .iter()
                    .filter(|(_, entry)| entry.info.project == project && entry.process.is_some())
                    .map(|(id, _)| *id)
                    .collect();
                for id in ids {
                    self.start_process(id, true);
                }
            }
            Command::StopAll(project) => {
                let ids: Vec<_> = self
                    .entries
                    .iter()
                    .filter(|(_, entry)| entry.info.project == project && entry.process.is_some())
                    .map(|(id, _)| *id)
                    .collect();
                for id in ids {
                    self.stop(id, false, false);
                }
            }
            Command::ProjectOpened(project) => {
                if self.opened.insert(project.clone()) && !self.recovery_pending {
                    self.autostart(&project);
                }
            }
            Command::ShowOrphans => {
                if let Some(store) = &self.store {
                    match store.survivors() {
                        Ok(orphans) => {
                            self.orphans = orphans;
                            if !self.orphans.is_empty() {
                                self.recovery_pending = true;
                                self.emit(CoreEvent::Orphans(self.orphans.clone()));
                            } else {
                                self.emit(CoreEvent::ConfigError(
                                    "No previous processes are still running".into(),
                                ));
                            }
                        }
                        Err(error) => self.emit(CoreEvent::ConfigError(error)),
                    }
                }
            }
            Command::ResolveOrphans { kill } => {
                self.recovery_pending = false;
                if kill {
                    self.capture_codex_orphan_jobs();
                    for orphan in &self.orphans {
                        if let Ok(members) = orphan.session.members() {
                            for member in members {
                                let _ = member.signal(libc::SIGTERM, orphan.session.leader.pid);
                            }
                        }
                    }
                    self.orphan_kill_at = Some(self.clock.now() + Duration::from_secs(3));
                    self.orphan_check_at = Some(self.clock.now());
                } else {
                    for project in self.opened.clone() {
                        self.autostart(&project);
                    }
                }
            }
            Command::ReloadConfig => self.load_config(),
            Command::AddProject(folder) => match config::append_project(&self.path, &folder) {
                Ok((config, id)) => {
                    self.config = config;
                    self.emit(CoreEvent::Config(self.config.clone()));
                    self.sync_processes();
                    self.emit(CoreEvent::ProjectAdded(id));
                }
                Err(error) => self.emit(CoreEvent::ConfigError(error)),
            },
            Command::AddProcess {
                project,
                name,
                command,
            } => match config::append_process(&self.path, &project, &name, &command) {
                Ok(config) => {
                    self.config = config;
                    self.emit(CoreEvent::Config(self.config.clone()));
                    self.sync_processes();
                    self.emit(CoreEvent::ProcessAdded(project));
                }
                Err(error) => self.emit(CoreEvent::ProcessAddFailed { project, error }),
            },
            Command::EditProcess {
                project,
                process,
                name,
                command,
                autostart,
            } => {
                match config::edit_process(
                    &self.path, &project, &process, &name, &command, autostart,
                ) {
                    Ok(config) => {
                        self.config = config;
                        self.emit(CoreEvent::Config(self.config.clone()));
                        self.sync_processes();
                        self.emit(CoreEvent::ProcessEdited { project, process });
                    }
                    Err(error) => self.emit(CoreEvent::ProcessAddFailed { project, error }),
                }
            }
            Command::NewShell { project, size } => self.new_shell(project, size, None),
            Command::NewTerminalPreset { project, name } => {
                let preset = self
                    .config
                    .projects
                    .iter()
                    .find(|p| p.id == project)
                    .and_then(|p| p.terminals.iter().find(|t| t.name == name))
                    .cloned();
                if let Some(preset) = preset {
                    self.new_shell(project, Dimensions::default(), Some(preset));
                } else {
                    self.emit(CoreEvent::ConfigError(
                        "Terminal preset is no longer configured".into(),
                    ));
                }
            }
            Command::Rename { id, name } => {
                let name: String = name
                    .trim()
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(100)
                    .collect();
                if !name.is_empty()
                    && !self.removing.contains(&id)
                    && let Some(entry) = self.entries.get_mut(&id)
                {
                    entry.info.name = name.clone();
                    let record = entry.agent.as_mut().map(|runtime| {
                        runtime.info.record.name = name;
                        runtime.info.record.clone()
                    });
                    if let (Some(store), Some(record)) = (&self.store, record)
                        && let Err(error) = store.save_agent(&record)
                    {
                        self.emit(CoreEvent::ConfigError(error));
                    }
                    self.changed(id);
                }
            }
            Command::Close { id, confirmed } => {
                if self
                    .entries
                    .get(&id)
                    .is_some_and(|entry| entry.process.is_some() || entry.agent.is_some())
                {
                    self.stop(id, false, false);
                    return;
                }
                if let Some(entry) = self.entries.get(&id)
                    && confirmed != Some(entry.info.launch)
                    && let Some(job) = entry
                        .session
                        .as_ref()
                        .and_then(TerminalSession::foreground_job)
                {
                    self.emit(CoreEvent::ConfirmClose {
                        id,
                        launch: entry.info.launch,
                        job,
                    });
                } else {
                    self.stop(id, true, false);
                }
            }
            Command::Restart(id) => {
                if self
                    .entries
                    .get(&id)
                    .is_some_and(|entry| entry.agent.is_some())
                {
                    self.resume_agent(id, None);
                    return;
                }
                if self.entries.get(&id).is_some_and(|entry| {
                    entry.process.is_some()
                        && entry.session.is_none()
                        && !entry.joining
                        && entry.exited
                }) {
                    self.start_process(id, true);
                } else {
                    self.stop(id, false, true);
                }
            }
            Command::Shutdown => {
                if self.shutdown_deadline.is_none() {
                    if let Some((_, stop)) = &self.meeting_stop {
                        let _ = stop.send(());
                    }
                    self.shutdown_deadline = Some(self.clock.now() + Duration::from_secs(15));
                    let ids: Vec<_> = self.entries.keys().copied().collect();
                    for id in ids {
                        self.stop(id, true, false);
                    }
                    self.emit(CoreEvent::ShutdownProgress {
                        remaining: self.entries.len(),
                    });
                }
            }
        }
    }
    fn list_prompts(&self, query: String, limit: usize, offset: usize) {
        match self
            .store
            .as_ref()
            .ok_or("Prompt store unavailable".to_string())
            .and_then(|store| store.list_prompts(&query, limit, offset))
        {
            Ok(items) => self.emit(CoreEvent::PromptsListed {
                query,
                offset,
                items,
            }),
            Err(error) => self.emit(CoreEvent::PromptError(error)),
        }
    }

    fn list_meetings(&self) {
        match self
            .store
            .as_ref()
            .ok_or("Meeting store unavailable".to_string())
            .and_then(Store::meetings)
        {
            Ok(meetings) => self.emit(CoreEvent::MeetingsListed(meetings)),
            Err(error) => self.emit(CoreEvent::MeetingError(error)),
        }
    }

    fn recover_meetings(&mut self) {
        let Some(store) = &self.store else { return };
        let Ok(meetings) = store.meetings() else {
            return;
        };
        for mut meeting in meetings {
            if matches!(
                meeting.status,
                meetings::Status::Starting
                    | meetings::Status::Recording
                    | meetings::Status::Stopping
                    | meetings::Status::Transcribing
                    | meetings::Status::GeneratingNotes
            ) {
                meeting.status = meetings::Status::NeedsAttention;
                meeting.error =
                    Some("Interrupted. Retry processing if the audio file is present.".into());
                let _ = store.save_meeting(&mut meeting);
            }
        }
        self.list_meetings();
    }

    fn start_meeting(&mut self, notes_delivery: meetings::NotesDelivery) {
        if !meetings::supported_os() {
            self.emit(CoreEvent::MeetingError(
                "Experimental meeting recording requires macOS 26 or later".into(),
            ));
            return;
        }
        if self.meeting_stop.is_some() {
            self.emit(CoreEvent::MeetingError(
                "A meeting is already recording".into(),
            ));
            return;
        }
        let result = (|| {
            let helper = meetings::helper_path()?;
            let root = self
                .path
                .parent()
                .ok_or("Config folder unavailable")?
                .join("meetings");
            let mut meeting = meetings::Meeting::new(root);
            meeting.notes_delivery = notes_delivery;
            let audio = meeting
                .audio_path
                .clone()
                .ok_or("Recording path unavailable")?;
            let microphone = meeting
                .microphone_path
                .clone()
                .ok_or("Microphone path unavailable")?;
            let folder = audio.parent().ok_or("Recording folder unavailable")?;
            std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
            self.store
                .as_ref()
                .ok_or("Meeting store unavailable")?
                .save_meeting(&mut meeting)?;
            let id = meeting.id.clone();
            let sender = self.sender.clone();
            let stop = meetings::start_recording(
                audio,
                microphone,
                helper,
                notes_delivery,
                move |update| {
                    let _ = sender.send(Message::Meeting {
                        id: id.clone(),
                        update,
                    });
                },
            );
            self.meeting_stop = Some((meeting.id.clone(), stop));
            self.emit(CoreEvent::MeetingUpdated(meeting));
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.emit(CoreEvent::MeetingError(error));
        }
    }

    fn stop_meeting(&mut self) {
        let Some((id, stop)) = &self.meeting_stop else {
            return;
        };
        let id = id.clone();
        let _ = stop.send(());
        self.edit_meeting(id, |meeting| {
            if matches!(
                meeting.status,
                meetings::Status::Starting | meetings::Status::Recording
            ) {
                meeting.status = meetings::Status::Stopping;
            }
            Ok(())
        });
    }

    fn meeting_update(&mut self, id: String, update: meetings::Update) {
        let result = (|| {
            let store = self.store.as_ref().ok_or("Meeting store unavailable")?;
            let mut meeting = store.meeting(&id)?.ok_or("Meeting no longer exists")?;
            match update {
                meetings::Update::Recording if meeting.status != meetings::Status::Stopping => {
                    meeting.status = meetings::Status::Recording
                }
                meetings::Update::Recording => {}
                meetings::Update::Stopped(duration) => {
                    meeting.duration_secs = duration;
                    meeting.status = meetings::Status::Transcribing;
                    if meeting
                        .audio_path
                        .as_ref()
                        .is_some_and(|path| !path.is_file())
                    {
                        meeting.audio_path = None;
                    }
                    if meeting
                        .microphone_path
                        .as_ref()
                        .is_some_and(|path| !path.is_file())
                    {
                        meeting.microphone_path = None;
                    }
                    if self
                        .meeting_stop
                        .as_ref()
                        .is_some_and(|(active, _)| active == &id)
                    {
                        self.meeting_stop = None;
                    }
                }
                meetings::Update::Transcribing => meeting.status = meetings::Status::Transcribing,
                meetings::Update::Transcript(text) => meeting.transcript = text,
                meetings::Update::GeneratingNotes => {
                    meeting.status = meetings::Status::GeneratingNotes
                }
                meetings::Update::Notes(text) => {
                    meeting.notes = text;
                    meeting.status = meetings::Status::Ready;
                    meeting.error = None;
                }
                meetings::Update::Ready => {
                    meeting.status = meetings::Status::Ready;
                    meeting.error = None;
                }
                meetings::Update::Error(error) => {
                    meeting.status = meetings::Status::NeedsAttention;
                    meeting.error = Some(error);
                    if self
                        .meeting_stop
                        .as_ref()
                        .is_some_and(|(active, _)| active == &id)
                    {
                        self.meeting_stop = None;
                    }
                }
            }
            store.save_meeting(&mut meeting)?;
            self.emit(CoreEvent::MeetingUpdated(meeting));
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.emit(CoreEvent::MeetingError(error));
        }
    }

    fn retry_meeting(&mut self, id: String) {
        let result = (|| {
            let store = self.store.as_ref().ok_or("Meeting store unavailable")?;
            let mut meeting = store.meeting(&id)?.ok_or("Meeting no longer exists")?;
            if !matches!(
                meeting.status,
                meetings::Status::Ready | meetings::Status::NeedsAttention
            ) {
                return Err("Wait for the meeting to finish processing".into());
            }
            let audio = meeting
                .audio_path
                .clone()
                .or_else(|| {
                    meeting
                        .microphone_path
                        .as_ref()
                        .map(|path| path.with_file_name("system.caf"))
                })
                .ok_or("This meeting has no audio to process")?;
            let microphone = meeting
                .microphone_path
                .clone()
                .unwrap_or_else(|| audio.with_file_name("microphone.caf"));
            if !audio.is_file() && !microphone.is_file() {
                return Err("This meeting has no audio to process".into());
            }
            let helper = meetings::helper_path()?;
            meeting.status = meetings::Status::Transcribing;
            meeting.error = None;
            let notes_delivery = meeting.notes_delivery;
            store.save_meeting(&mut meeting)?;
            self.emit(CoreEvent::MeetingUpdated(meeting));
            let sender = self.sender.clone();
            meetings::retry(audio, microphone, helper, notes_delivery, move |update| {
                let _ = sender.send(Message::Meeting {
                    id: id.clone(),
                    update,
                });
            });
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.emit(CoreEvent::MeetingError(error));
        }
    }

    fn edit_meeting(
        &mut self,
        id: String,
        edit: impl FnOnce(&mut meetings::Meeting) -> Result<(), String>,
    ) {
        let result = (|| {
            let store = self.store.as_ref().ok_or("Meeting store unavailable")?;
            let mut meeting = store.meeting(&id)?.ok_or("Meeting no longer exists")?;
            edit(&mut meeting)?;
            store.save_meeting(&mut meeting)?;
            self.emit(CoreEvent::MeetingUpdated(meeting));
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.emit(CoreEvent::MeetingError(error));
        }
    }

    fn delete_meeting_audio(&mut self, id: String) {
        self.edit_meeting(id, |meeting| {
            if !matches!(
                meeting.status,
                meetings::Status::Ready | meetings::Status::NeedsAttention
            ) {
                return Err("Wait for processing to finish before deleting audio".into());
            }
            if let Some(path) = &meeting.audio_path
                && path.exists()
            {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
            }
            if let Some(path) = &meeting.microphone_path
                && path.exists()
            {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
            }
            meeting.audio_path = None;
            meeting.microphone_path = None;
            Ok(())
        });
    }

    fn open_meeting_audio(&self, id: String, microphone: bool) {
        let result = (|| {
            let meeting = self
                .store
                .as_ref()
                .ok_or("Meeting store unavailable")?
                .meeting(&id)?
                .ok_or("Meeting no longer exists")?;
            let path = (if microphone {
                meeting.microphone_path
            } else {
                meeting.audio_path
            })
            .filter(|path| path.is_file())
            .ok_or("This meeting has no audio for that source")?;
            std::process::Command::new("/usr/bin/open")
                .arg(path)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.emit(CoreEvent::MeetingError(error));
        }
    }

    fn delete_meeting(&mut self, id: String) {
        let result = (|| {
            if self
                .meeting_stop
                .as_ref()
                .is_some_and(|(active, _)| active == &id)
            {
                return Err("Stop recording before deleting the meeting".into());
            }
            let store = self.store.as_ref().ok_or("Meeting store unavailable")?;
            let meeting = store.meeting(&id)?.ok_or("Meeting no longer exists")?;
            if !matches!(
                meeting.status,
                meetings::Status::Ready | meetings::Status::NeedsAttention
            ) {
                return Err("Wait for processing to finish before deleting the meeting".into());
            }
            let id = uuid::Uuid::parse_str(&id)
                .map_err(|e| e.to_string())?
                .to_string();
            let folder = self
                .path
                .parent()
                .ok_or("Config folder unavailable")?
                .join("meetings")
                .join(&id);
            if folder.is_dir() {
                std::fs::remove_dir_all(&folder).map_err(|e| e.to_string())?;
            }
            store.delete_meeting(&id)?;
            self.emit(CoreEvent::MeetingDeleted(id));
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.emit(CoreEvent::MeetingError(error));
        }
    }

    fn mcp_request(&mut self, request: mcp::Request) {
        let authorized = self.entries.iter().find_map(|(id, entry)| {
            let runtime = entry.agent.as_ref()?;
            (runtime.info.record.id == request.envelope.agent_id
                && entry.info.launch.0 == request.envelope.launch
                && runtime.token == request.envelope.token
                && entry.info.status == SessionStatus::Running
                && entry.stopping.is_none()
                && !entry.exited
                && !self.removing.contains(id))
            .then(|| {
                (
                    runtime.info.record.role.clone(),
                    runtime.info.record.name.clone(),
                )
            })
        });
        let Some((role, agent_name)) = authorized else {
            let _ = request.reply.send(mcp::Reply {
                result: None,
                error: Some("Agent session is no longer authorized".into()),
            });
            return;
        };
        let write = matches!(
            request.envelope.operation,
            mcp::Operation::Write { .. }
                | mcp::Operation::Delete { .. }
                | mcp::Operation::EditMeeting { .. }
        );
        if write && role != agent::Role::Primary {
            let _ = request.reply.send(mcp::Reply {
                result: None,
                error: Some("Read-only agents cannot change library content".into()),
            });
            return;
        }
        let Some(store) = &self.store else {
            let _ = request.reply.send(mcp::Reply {
                result: None,
                error: Some("Library store unavailable".into()),
            });
            return;
        };
        if let mcp::Operation::Delete { ref id, revision } = request.envelope.operation {
            let result = prompts::validate_delete(id, revision)
                .and_then(|()| store.read_prompt(id)?.ok_or("Prompt not found".into()))
                .and_then(|prompt| {
                    if prompt.revision != revision {
                        Err("Prompt changed; reload before deleting".into())
                    } else {
                        Ok(prompt)
                    }
                });
            match result {
                Ok(prompt) => {
                    let request_id = self.next_prompt_delete;
                    self.next_prompt_delete += 1;
                    self.pending_prompt_deletes.insert(
                        request_id,
                        (request, self.clock.now() + Duration::from_secs(90)),
                    );
                    self.emit(CoreEvent::PromptDeleteApproval {
                        request_id,
                        title: prompt.title,
                        agent: agent_name,
                    });
                }
                Err(error) => {
                    let _ = request.reply.send(mcp::Reply {
                        result: None,
                        error: Some(error),
                    });
                }
            }
            return;
        }
        let result: Result<serde_json::Value, String> = match &request.envelope.operation {
            mcp::Operation::List {
                query,
                limit,
                offset,
            } => store
                .list_prompts(query, *limit, *offset)
                .and_then(|items| serde_json::to_value(items).map_err(|e| e.to_string())),
            mcp::Operation::Read { id } => store
                .read_prompt(id)
                .and_then(|prompt| prompt.ok_or("Prompt not found".into()))
                .and_then(|prompt| serde_json::to_value(prompt).map_err(|e| e.to_string())),
            mcp::Operation::Write { write } => store.write_prompt(write).and_then(|prompt| {
                self.emit(CoreEvent::PromptSaved {
                    prompt: prompt.clone(),
                    from_ui: false,
                });
                serde_json::to_value(prompt).map_err(|e| e.to_string())
            }),
            mcp::Operation::ListMeetings {
                query,
                limit,
                offset,
            } => store.meetings().map(|meetings| {
                let query = query.trim().to_lowercase();
                let items: Vec<_> = meetings
                    .iter()
                    .filter(|meeting| {
                        query.is_empty()
                            || meeting.title.to_lowercase().contains(&query)
                            || meeting.notes.to_lowercase().contains(&query)
                    })
                    .skip((*offset).min(100_000))
                    .take((*limit).clamp(1, 100))
                    .map(mcp::meeting_summary)
                    .collect();
                serde_json::json!(items)
            }),
            mcp::Operation::ReadMeeting { id } => store
                .meeting(id)
                .and_then(|meeting| meeting.ok_or("Meeting not found".into()))
                .and_then(|meeting| {
                    if meeting.notes.len() > mcp::MAX_MCP_NOTES {
                        Err("Meeting notes exceed the MCP read limit of 128 KiB".into())
                    } else {
                        Ok(mcp::meeting_detail(&meeting))
                    }
                }),
            mcp::Operation::EditMeeting { edit } => edit.validate().and_then(|()| {
                let mut meeting = store
                    .meeting(&edit.id)?
                    .ok_or("Meeting not found".to_string())?;
                if meeting.revision != edit.expected_revision {
                    return Err("Meeting changed; read_meeting again before editing".into());
                }
                if edit.notes.is_some()
                    && !matches!(
                        meeting.status,
                        meetings::Status::Ready | meetings::Status::NeedsAttention
                    )
                {
                    return Err("Wait for meeting processing to finish before editing notes".into());
                }
                if let Some(title) = &edit.title {
                    meeting.title = title.trim().to_string();
                }
                if let Some(notes) = &edit.notes {
                    meeting.notes = notes.clone();
                }
                store.save_meeting(&mut meeting)?;
                self.emit(CoreEvent::MeetingUpdated(meeting.clone()));
                Ok(mcp::meeting_detail(&meeting))
            }),
            mcp::Operation::Delete { .. } => unreachable!(),
        };
        if result.is_ok() && matches!(request.envelope.operation, mcp::Operation::Write { .. }) {
            self.list_prompts(String::new(), 100, 0);
        }
        let _ = request.reply.send(match result {
            Ok(value) => mcp::Reply {
                result: Some(value),
                error: None,
            },
            Err(error) => mcp::Reply {
                result: None,
                error: Some(error),
            },
        });
    }

    fn resolve_prompt_delete(&mut self, request_id: u64, approved: bool) {
        let Some((request, _)) = self.pending_prompt_deletes.remove(&request_id) else {
            return;
        };
        if !approved {
            let _ = request.reply.send(mcp::Reply {
                result: None,
                error: Some("User cancelled prompt deletion".into()),
            });
            return;
        }
        let mcp::Operation::Delete { id, revision } = request.envelope.operation else {
            return;
        };
        let result = self
            .store
            .as_ref()
            .ok_or("Prompt store unavailable".to_string())
            .and_then(|store| store.delete_prompt(&id, revision));
        if result.is_ok() {
            self.emit(CoreEvent::PromptDeleted(id.clone()));
            self.list_prompts(String::new(), 100, 0);
        }
        let _ = request.reply.send(match result {
            Ok(()) => mcp::Reply {
                result: Some(serde_json::json!({"deleted":id})),
                error: None,
            },
            Err(error) => mcp::Reply {
                result: None,
                error: Some(error),
            },
        });
    }

    /// Codex tool PTYs use their own SIDs. Capture live descendant sessions
    /// before killing the TUI, and journal each birth identity so a crash during
    /// escalation can recover it. No ancestry polling is needed while idle.
    fn track_codex_descendants(&mut self, id: SessionId) {
        let owner = self.owner(id);
        let Some(entry) = self.entries.get_mut(&id) else {
            return;
        };
        if !entry
            .agent
            .as_ref()
            .is_some_and(|runtime| runtime.info.record.provider == agent::Provider::Codex)
            || entry.stopping.as_ref().is_some_and(|plan| plan.stage > 0)
        {
            return;
        }
        let Some(session) = &mut entry.session else {
            return;
        };
        let discovered = match session.owned.descendant_sessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!(%error, "Cannot capture Codex child sessions");
                return;
            }
        };
        let mut error = None;
        // Also retry identities already held in memory: a prior SQLite error
        // must not silently prevent later crash-journal persistence.
        let mut identities = session.extra_sessions().to_vec();
        for owned in &discovered {
            if !identities.iter().any(|old| old.leader == owned.leader) {
                identities.push(*owned);
            }
        }
        for owned in identities {
            if let Some(store) = &self.store {
                match store.contains(owned) {
                    Ok(true) => {}
                    Ok(false) => {
                        if let Err(failure) = store.insert(owned, entry.info.launch.0, &owner) {
                            error = Some(failure);
                        }
                    }
                    Err(failure) => error = Some(failure),
                }
            }
        }
        session.track_sessions(discovered);
        if let Some(error) = error {
            self.emit(CoreEvent::ConfigError(error));
        }
    }
    fn codex_jobs_need_scan(&self) -> bool {
        self.entries.values().any(|entry| {
            entry.stopping.is_none()
                && entry.session.is_some()
                && entry.agent.as_ref().is_some_and(|runtime| {
                    runtime.info.record.provider == agent::Provider::Codex
                        && (runtime.decay_at.is_some()
                            || entry
                                .session
                                .as_ref()
                                .is_some_and(|session| !session.extra_sessions().is_empty()))
                })
        })
    }
    fn arm_codex_scan(&mut self) {
        if !self.codex_jobs_need_scan() {
            self.codex_scan_at = None;
        } else if self.codex_scan_at.is_none() {
            self.codex_scan_at = Some(self.clock.now() + Duration::from_secs(1));
        }
    }
    /// Best-effort crash journal while a Codex turn or background job is live.
    /// There is no scan deadline with idle agents and no tracked jobs. A process
    /// born and orphaned between scans remains an explicitly documented gap.
    fn scan_codex_jobs(&mut self, now: Duration) {
        if !self.codex_scan_at.is_some_and(|deadline| now >= deadline) {
            return;
        }
        let ids: Vec<_> =
            self.entries
                .iter()
                .filter(|(_, entry)| {
                    entry.stopping.is_none()
                        && entry.session.is_some()
                        && entry.agent.as_ref().is_some_and(|runtime| {
                            runtime.info.record.provider == agent::Provider::Codex
                                && (runtime.decay_at.is_some()
                                    || entry.session.as_ref().is_some_and(|session| {
                                        !session.extra_sessions().is_empty()
                                    }))
                        })
                })
                .map(|(id, _)| *id)
                .collect();
        let mut prune_error = None;
        for id in ids {
            self.track_codex_descendants(id);
            let session = self.entries.get_mut(&id).unwrap().session.as_mut().unwrap();
            let expired: Vec<_> = session
                .extra_sessions()
                .iter()
                .copied()
                .filter(|owned| owned.members().is_ok_and(|members| members.is_empty()))
                .collect();
            for owned in expired {
                if let Some(store) = &self.store
                    && let Err(error) = store.remove(owned)
                {
                    prune_error = Some(error);
                    continue;
                }
                session.forget_session(owned.leader);
            }
        }
        if let Some(error) = prune_error {
            self.emit(CoreEvent::ConfigError(error));
        }
        self.codex_scan_at = self
            .codex_jobs_need_scan()
            .then_some(now + Duration::from_secs(1));
    }
    fn stop(&mut self, id: SessionId, close: bool, restart: bool) {
        if self.entries.contains_key(&id) {
            self.track_codex_descendants(id);
        }
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.close |= close;
            entry.restart = restart && !entry.close;
            entry
                .stopping
                .get_or_insert_with(|| StopPlan::new(self.clock.now()));
            entry.info.status = SessionStatus::Stopping;
            if let Some(process) = &mut entry.process {
                process.stop();
                let plan = StopPlan::for_process(self.clock.now(), &process.spec);
                // Repeated clicks never extend an already-running stop deadline.
                if entry.stopping.as_ref().is_some_and(|stop| stop.stage == 0) {
                    entry.stopping = Some(plan);
                }
            }
            if entry.agent.is_some() && entry.stopping.as_ref().is_some_and(|stop| stop.stage == 0)
            {
                entry.stopping = Some(StopPlan::for_agent(self.clock.now()));
            }
            self.changed(id);
        }
    }
    fn spawn(&mut self, id: SessionId) {
        if self.probe.is_none() && self.store.is_none() {
            let entry = self.entries.get_mut(&id).expect("registered spawn");
            let error =
                "Session ledger unavailable; restart DevDesk after repairing state.db".to_string();
            entry.info.status = SessionStatus::Failed(error.clone());
            entry.exited = true;
            entry.drained = true;
            if let Some(runtime) = &mut entry.agent {
                runtime.exited();
            }
            if let Some(process) = &mut entry.process {
                process.fail(error);
            }
            self.changed(id);
            return;
        }
        let entry = self.entries.get_mut(&id).expect("registered before spawn");
        let Some(project) = self
            .config
            .projects
            .iter()
            .find(|project| project.id == entry.info.project)
        else {
            entry.info.status = SessionStatus::Failed("Project was removed from config".into());
            if let Some(runtime) = &mut entry.agent {
                runtime.exited();
            }
            if let Some(process) = &mut entry.process {
                process.fail("Project was removed from config".into());
            }
            entry.exited = true;
            entry.drained = true;
            entry.stopping = None;
            self.changed(id);
            return;
        };
        let project = project.clone();
        let process_spec = if entry.process.is_some() {
            entry.desired.clone()
        } else {
            None
        };
        if entry.process.is_some() && process_spec.is_none() {
            entry.info.status = SessionStatus::Failed("Process was removed from config".into());
            if let Some(process) = &mut entry.process {
                process.fail("Process was removed from config".into());
            }
            entry.exited = true;
            entry.drained = true;
            self.changed(id);
            return;
        }
        if let (Some(process), Some(spec)) = (&mut entry.process, &process_spec) {
            entry.info.name = spec.name.clone();
            process.spec = spec.clone();
        }
        entry.info.config_changed = false;
        let history = if entry.process.is_some() {
            entry.history.clone()
        } else {
            None
        };
        let separator = entry
            .process
            .as_ref()
            .map(|process| {
                format!(
                    "── restarted after exit {} ──",
                    process
                        .last_exit
                        .map_or_else(|| "signal / stop".into(), |code| code.to_string())
                )
            })
            .unwrap_or_default();
        let cwd = entry.cwd.clone().unwrap_or_else(|| project.path.clone());
        entry.cwd = Some(cwd.clone());
        let agent_launch = entry.agent.as_ref().map(|runtime| {
            (
                runtime.info.record.clone(),
                runtime.request.clone(),
                runtime.token.clone(),
            )
        });
        let socket = self.hooks.as_ref().map(|server| server.path.clone());
        let size = entry.size;
        let lines = self.config.settings.scrollback_lines;
        let launch = LaunchId(self.next_launch);
        self.next_launch += 1;
        entry.info.pid = None;
        entry.info.launch = launch;
        entry.info.status = SessionStatus::Starting;
        entry.drained = false;
        entry.exited = false;
        entry.stopping = None;
        entry.restart = false;
        self.changed(id);
        let sender = self.sender.clone();
        let probe = self.probe.clone();
        let result_sender = sender.clone();
        let helper = self.helper.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("devdesk-spawn".into())
            .spawn(move || {
                let environment = ShellEnvironment::shared();
                let mut warning = environment.warning.clone();
                let mut spawn_started = None;
                let result = (|| {
                    let (program, args) = probe_command(
                        if probe.as_deref() == Some("many") {
                            Some(if id.0 == crate::perf::noisy_count() as u64 + 1 {
                                "echo"
                            } else {
                                "yes"
                            })
                        } else {
                            probe.as_deref()
                        },
                        &environment,
                    )?;
                    let helper = if let Some(helper) = &helper {
                        helper.clone()
                    } else {
                        std::env::current_exe().map_err(|e| e.to_string())?
                    };
                    let mut spec = if let Some((record, request, token)) = &agent_launch {
                        let context = agent::Context {
                            record,
                            request,
                            launch: launch.0,
                            token,
                            socket: socket.as_deref().ok_or(
                                "Hook server unavailable; refusing untracked agent launch",
                            )?,
                            helper: &helper,
                            lines,
                        };
                        match record.provider {
                            agent::Provider::Claude => {
                                agent::ClaudeProvider.build_launch(&context, &environment)?
                            }
                            agent::Provider::Codex => {
                                crate::codex::CodexProvider.build_launch(&context, &environment)?
                            }
                            provider => agent::InteractiveProvider(provider)
                                .build_launch(&context, &environment)?,
                        }
                    } else if let Some(process_spec) = &process_spec {
                        process::launch(&project, process_spec, &environment, lines)
                    } else {
                        SpawnSpec {
                            program,
                            args,
                            cwd,
                            env: environment.variables.clone(),
                            scrollback_lines: lines,
                        }
                    };
                    if let Some((record, _, _)) = &agent_launch
                        && let Some(version) =
                            agent::version_warning(record.provider, &spec.program, &environment)
                    {
                        warning = Some(warning.take().map_or_else(
                            || version.clone(),
                            |warning| format!("{warning}\n{version}"),
                        ));
                    }
                    let env = &mut spec.env;
                    env.insert("DEVDESK_SESSION_ID".into(), id.0.to_string());
                    env.insert("DEVDESK_LAUNCH_ID".into(), launch.0.to_string());
                    spawn_started = probe.as_ref().map(|_| Instant::now());
                    TerminalSession::spawn_reusing(
                        spec,
                        size,
                        Some(Box::new(move |signal| {
                            let _ = sender.send(Message::Lifecycle { id, launch, signal });
                        })),
                        history,
                        &separator,
                    )
                    .map_err(|error| error.to_string())
                })();
                if probe.is_some()
                    && let Ok((session, _)) = &result
                {
                    eprintln!(
                        "devdesk-owned sid={} started_us={}",
                        session.pid, session.owned.leader.started_us
                    );
                }
                let _ = result_sender.send(Message::Spawned {
                    id,
                    launch,
                    result: Box::new(result),
                    warning,
                    spawn_started,
                });
            })
        {
            self.message(Message::Spawned {
                id,
                launch,
                result: Box::new(Err(error.to_string())),
                warning: None,
                spawn_started: None,
            });
        }
    }
    fn key(&self, entry: &Entry) -> String {
        if let Some(runtime) = &entry.agent {
            format!("agent:{}", runtime.info.record.id)
        } else if let Some(process) = &entry.info.process {
            format!("process:{}/{}", entry.info.project, process)
        } else {
            format!("shell:{}", entry.info.id.0)
        }
    }
    fn flush_layout(&mut self) {
        self.layout_due = None;
        if let Some(layout) = self.pending_layout.take()
            && let Some(store) = &self.store
            && let Err(error) = store.save_layout(&layout)
        {
            self.emit(CoreEvent::ConfigError(error));
        }
    }
    fn agent_entry(&mut self, record: agent::Record, request: agent::Request) -> SessionId {
        let id = SessionId(self.next_id);
        self.next_id += 1;
        let runtime = agent::Runtime::new(record, request);
        self.entries.insert(
            id,
            Entry {
                info: SessionInfo {
                    agent: Some(runtime.info.clone()),
                    process: None,
                    config_changed: false,
                    pid: None,
                    id,
                    launch: LaunchId(0),
                    project: runtime.info.record.project.clone(),
                    name: runtime.info.record.name.clone(),
                    status: SessionStatus::Exited { code: None },
                },
                cwd: Some(runtime.info.record.cwd.clone()),
                agent: Some(runtime),
                process: None,
                desired: None,
                history: None,
                joining: false,
                session: None,
                size: Dimensions::default(),
                stopping: None,
                close: false,
                restart: false,
                drained: true,
                exited: true,
            },
        );
        id
    }
    fn remove_session(&mut self, id: SessionId) {
        let Some(entry) = self.entries.get(&id) else {
            return;
        };
        let children: Vec<_> = entry
            .agent
            .as_ref()
            .map(|runtime| &runtime.info.record.id)
            .map(|parent| {
                self.entries
                    .iter()
                    .filter(|(_, entry)| {
                        entry
                            .agent
                            .as_ref()
                            .is_some_and(|agent| agent.info.record.parent.as_ref() == Some(parent))
                    })
                    .map(|(id, _)| *id)
                    .collect()
            })
            .unwrap_or_default();
        let result = if let Some(runtime) = &entry.agent {
            self.store
                .as_ref()
                .ok_or("Agent store unavailable".into())
                .and_then(|store| store.delete_agent(&runtime.info.record.id))
        } else if let Some(process) = &entry.info.process {
            config::remove(&self.path, &entry.info.project, Some(process)).map(|config| {
                self.config = config;
                self.emit(CoreEvent::Config(self.config.clone()));
                self.sync_processes();
            })
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.emit(CoreEvent::ConfigError(error));
            return;
        }
        for child in children {
            self.removing.insert(child);
            self.stop(child, true, false);
        }
        self.removing.insert(id);
        self.stop(id, true, false);
    }
    fn remove_project(&mut self, project: String) {
        let config = match config::remove(&self.path, &project, None) {
            Ok(config) => config,
            Err(error) => {
                self.emit(CoreEvent::ConfigError(error));
                return;
            }
        };
        self.config = config;
        // Exclude late hooks before deleting records, so they cannot resurrect
        // a removed conversation while its PTY is shutting down.
        let ids: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.info.project == project)
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            self.removing.insert(*id);
        }
        if let Some(store) = &self.store
            && let Err(error) = store.delete_project_agents(&project)
        {
            self.emit(CoreEvent::ConfigError(error));
        }
        self.emit(CoreEvent::Config(self.config.clone()));
        self.sync_processes();
        for id in ids {
            self.stop(id, true, false);
        }
        self.emit(CoreEvent::ProjectRemoved(project));
    }
    fn new_shell(
        &mut self,
        project: String,
        size: Dimensions,
        preset: Option<config::TerminalSpec>,
    ) {
        if !self.config.projects.iter().any(|item| item.id == project) {
            self.emit(CoreEvent::ConfigError(
                "Choose an existing project before opening a terminal".into(),
            ));
            return;
        }
        let id = SessionId(self.next_id);
        self.next_id += 1;
        self.entries.insert(
            id,
            Entry {
                agent: None,
                cwd: preset.as_ref().and_then(|p| p.cwd.clone()),
                process: None,
                desired: None,
                history: None,
                joining: false,
                info: SessionInfo {
                    agent: None,
                    process: None,
                    config_changed: false,
                    pid: None,
                    id,
                    launch: LaunchId(0),
                    project,
                    name: preset
                        .map(|p| p.name)
                        .unwrap_or_else(|| format!("Terminal {}", id.0)),
                    status: SessionStatus::Starting,
                },
                session: None,
                size,
                stopping: None,
                close: false,
                restart: false,
                drained: false,
                exited: false,
            },
        );
        self.spawn(id);
    }
    fn new_agent(
        &mut self,
        project: String,
        picker: bool,
        provider: agent::Provider,
        customization: Option<String>,
        launch_args: Vec<String>,
    ) {
        if !provider.tracks_sessions() && (picker || customization.is_some()) {
            self.emit(CoreEvent::ConfigError(
                "This agent supports interactive launch and restart only".into(),
            ));
            return;
        }
        if self.recovery_pending || self.orphan_check_at.is_some() {
            self.emit(CoreEvent::ConfigError(
                "Resolve previous sessions before launching an agent".into(),
            ));
            return;
        }
        let Some(project) = self
            .config
            .projects
            .iter()
            .find(|candidate| candidate.id == project)
        else {
            return;
        };
        let customization = match customization {
            Some(request) => match crate::customization::Draft::prepare(&self.path, &request) {
                Ok(draft) => Some(draft),
                Err(error) => {
                    self.emit(CoreEvent::ConfigError(error));
                    return;
                }
            },
            None => None,
        };
        let record = agent::Record {
            id: uuid::Uuid::new_v4().to_string(),
            project: project.id.clone(),
            name: if customization.is_some() {
                format!("Customize DevDesk · {}", provider.label())
            } else if picker {
                format!("{} · choose session", provider.label())
            } else {
                provider.label().into()
            },
            cwd: customization
                .as_ref()
                .map(|(cwd, _)| cwd.clone())
                .unwrap_or_else(|| project.path.clone()),
            role: agent::Role::Primary,
            provider,
            launch_args,
            parent: None,
            snapshot_at: None,
            session_id: (!picker && provider == agent::Provider::Claude)
                .then(|| uuid::Uuid::new_v4().to_string()),
        };
        if let Err(error) = agent::launch_args(&record) {
            self.emit(CoreEvent::ConfigError(error));
            return;
        }
        let result = self
            .store
            .as_ref()
            .ok_or("Agent store unavailable".into())
            .and_then(|store| store.save_agent(&record));
        if let Err(error) = result {
            self.emit(CoreEvent::ConfigError(error));
            return;
        }
        let id = self.agent_entry(
            record,
            if let Some((_, prompt)) = customization {
                agent::Request::Prompt(prompt)
            } else if picker {
                agent::Request::ResumePicker
            } else {
                agent::Request::New
            },
        );
        self.spawn(id);
    }
    fn fork_agent(&mut self, parent: SessionId, question: String) {
        let question = question.trim();
        if question.is_empty() || question.len() > 8192 || question.contains(['\n', '\r']) {
            self.emit(CoreEvent::ConfigError(
                "Enter a one-line question (up to 8 KiB)".into(),
            ));
            return;
        }
        if self.recovery_pending
            || self.orphan_check_at.is_some()
            || self.removing.contains(&parent)
        {
            self.emit(CoreEvent::ConfigError(
                "Resolve previous sessions before asking a question".into(),
            ));
            return;
        }
        let Some(entry) = self.entries.get(&parent) else {
            return;
        };
        let Some(parent) = entry.agent.as_ref() else {
            self.emit(CoreEvent::ConfigError(
                "Select a running agent to ask a question".into(),
            ));
            return;
        };
        let record = &parent.info.record;
        if !record.provider.tracks_sessions() {
            self.emit(CoreEvent::ConfigError(
                "Side conversations are available for Claude and Codex agents".into(),
            ));
            return;
        }
        if record.role != agent::Role::Primary
            || entry.info.status != SessionStatus::Running
            || parent.info.status == agent::Status::Launching
        {
            self.emit(CoreEvent::ConfigError(
                "Questions need a running primary agent".into(),
            ));
            return;
        }
        let Some(parent_session_id) = record.session_id.clone() else {
            self.emit(CoreEvent::ConfigError(
                "The agent has not reported a session ID yet".into(),
            ));
            return;
        };
        let record = agent::Record {
            id: uuid::Uuid::new_v4().to_string(),
            project: record.project.clone(),
            name: question.chars().take(48).collect(),
            cwd: record.cwd.clone(),
            role: agent::Role::Side,
            provider: record.provider,
            launch_args: vec![],
            session_id: None,
            parent: Some(record.id.clone()),
            snapshot_at: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            ),
        };
        // Persist the role before starting any worker. Resume can only recover
        // this role; neither a UI action nor a launch request can upgrade it.
        if let Err(error) = self
            .store
            .as_ref()
            .ok_or("Agent store unavailable".into())
            .and_then(|store| store.save_agent(&record))
        {
            self.emit(CoreEvent::ConfigError(error));
            return;
        }
        let id = self.agent_entry(
            record,
            agent::Request::Fork {
                parent_session_id,
                question: question.into(),
            },
        );
        self.spawn(id);
    }
    fn resume_agent(&mut self, id: SessionId, args: Option<Vec<String>>) {
        if self.removing.contains(&id) {
            return;
        }
        if self.recovery_pending || self.orphan_check_at.is_some() {
            self.emit(CoreEvent::ConfigError(
                "Resolve previous sessions before resuming an agent".into(),
            ));
            return;
        }
        if let Some(runtime) = self.entries.get(&id).and_then(|entry| entry.agent.as_ref()) {
            let owner = format!("agent:{}", runtime.info.record.id);
            if self.orphans.iter().any(|orphan| {
                orphan.owner == owner
                    && !orphan
                        .session
                        .members()
                        .is_ok_and(|members| members.is_empty())
            }) {
                self.emit(CoreEvent::ConfigError(
                    "Previous agent instance is still running; recover it before resuming".into(),
                ));
                return;
            }
        }
        let Some(entry) = self.entries.get_mut(&id) else {
            return;
        };
        let Some(runtime) = &mut entry.agent else {
            return;
        };
        if entry.session.is_some() || entry.joining || !entry.exited {
            self.emit(CoreEvent::ConfigError(
                "Stop the agent before resuming this conversation".into(),
            ));
            return;
        }
        if let Some(args) = args {
            if runtime.info.record.role != agent::Role::Primary {
                self.emit(CoreEvent::ConfigError(
                    "Side conversation launch arguments cannot be customized".into(),
                ));
                return;
            }
            let mut record = runtime.info.record.clone();
            record.launch_args = args;
            let result = agent::launch_args(&record).and_then(|_| {
                self.store
                    .as_ref()
                    .ok_or("Agent store unavailable".into())
                    .and_then(|store| store.save_agent(&record))
            });
            if let Err(error) = result {
                self.emit(CoreEvent::ConfigError(error));
                return;
            }
            runtime.info.record = record;
        }
        runtime.request = if !runtime.info.record.provider.tracks_sessions() {
            agent::Request::New
        } else if runtime.info.record.session_id.is_some() {
            agent::Request::Resume
        } else {
            agent::Request::ResumePicker
        };
        runtime.launching();
        entry.close = false;
        self.spawn(id);
    }
    fn agent_input(&mut self, id: SessionId, launch: LaunchId, enter: Option<bool>) {
        let Some(entry) = self.entries.get_mut(&id) else {
            return;
        };
        if entry.info.launch != launch || entry.exited || self.removing.contains(&id) {
            return;
        }
        let Some(runtime) = &mut entry.agent else {
            return;
        };
        let previous = runtime.info.status.clone();
        match enter {
            Some(enter) => runtime.input(enter, self.clock.now()),
            None => runtime.bell(),
        }
        if previous != runtime.info.status {
            self.changed(id);
        }
        self.arm_codex_scan();
    }
    fn agent_exit_summary(&mut self, id: SessionId, launch: LaunchId, session_id: String) {
        if uuid::Uuid::parse_str(&session_id).is_err() || self.removing.contains(&id) {
            return;
        }
        let Some(entry) = self.entries.get_mut(&id) else {
            return;
        };
        let Some(runtime) = &mut entry.agent else {
            return;
        };
        if entry.info.launch != launch
            || runtime.info.record.provider != agent::Provider::Codex
            || runtime.info.record.session_id.is_some()
        {
            return;
        }
        if let agent::Request::Fork {
            parent_session_id, ..
        } = &runtime.request
            && *parent_session_id == session_id
        {
            return;
        }
        runtime.info.record.session_id = Some(session_id);
        if let Some(store) = &self.store
            && let Err(error) = store.save_agent(&runtime.info.record)
        {
            self.emit(CoreEvent::ConfigError(error));
        }
        self.changed(id);
    }
    fn decay_agents(&mut self) {
        let now = self.clock.now();
        let mut changed = vec![];
        for (id, entry) in &mut self.entries {
            let Some(runtime) = &mut entry.agent else {
                continue;
            };
            if !runtime.decay_at.is_some_and(|deadline| now >= deadline) {
                continue;
            }
            let age = entry
                .session
                .as_ref()
                .map(|session| session.handle.output_age())
                .unwrap_or(Duration::from_secs(600));
            let remaining = Duration::from_secs(600).saturating_sub(age);
            if remaining.is_zero() {
                let previous = runtime.info.status.clone();
                runtime.expire_inferred();
                if previous != runtime.info.status {
                    changed.push(*id);
                }
            } else {
                runtime.decay_at = Some(now + remaining);
            }
        }
        for id in changed {
            self.changed(id);
        }
    }
    fn hook_messages(&mut self) {
        let messages: Vec<_> = self
            .hooks
            .as_ref()
            .map(|server| server.events.try_iter().collect())
            .unwrap_or_default();
        for event in messages {
            self.hook(event);
        }
    }
    fn hook(&mut self, event: ipc::HookEvent) {
        let Some(id) = self
            .entries
            .iter()
            .find(|(_, entry)| {
                entry
                    .agent
                    .as_ref()
                    .is_some_and(|runtime| runtime.info.record.id == event.agent_id)
            })
            .map(|(id, _)| *id)
        else {
            return;
        };
        if self.removing.contains(&id) {
            return;
        }
        let entry = self.entries.get_mut(&id).expect("registered agent");
        let runtime = entry.agent.as_mut().expect("agent identity");
        if entry.info.launch.0 != event.launch || runtime.token != event.token || entry.exited {
            return;
        }
        let now = self.clock.now();
        if now.saturating_sub(runtime.rate_window) >= Duration::from_secs(1) {
            runtime.rate_window = now;
            runtime.rate_count = 0;
        }
        runtime.rate_count += 1;
        if runtime.rate_count > 100 {
            return;
        }
        if let agent::Request::Fork {
            parent_session_id, ..
        } = &runtime.request
            && runtime.info.record.session_id.is_none()
            && (event.event == "SessionStart" || event.event == "agent-turn-complete")
            && (event.session_id.is_none()
                || (runtime.info.record.provider == agent::Provider::Claude
                    && event.source.as_deref() != Some("fork"))
                || event.session_id.as_ref() == Some(parent_session_id))
        {
            self.emit(CoreEvent::ConfigError(
                "The CLI did not report a distinct fork identity; stopping the side conversation"
                    .into(),
            ));
            self.stop(id, false, false);
            return;
        }
        let previous = runtime.info.record.session_id.clone();
        runtime.apply(&event);
        let capture_jobs = runtime.info.record.provider == agent::Provider::Codex
            && event.event == "agent-turn-complete"
            && event.session_id.is_some()
            && event.session_id == runtime.info.record.session_id;
        let record =
            (previous != runtime.info.record.session_id).then(|| runtime.info.record.clone());
        if let (Some(store), Some(record)) = (&self.store, record)
            && let Err(error) = store.save_agent(&record)
        {
            self.emit(CoreEvent::ConfigError(error));
        }
        if capture_jobs {
            self.track_codex_descendants(id);
        }
        self.arm_codex_scan();
        self.changed(id);
    }
    fn restore(&mut self) {
        if self.restored || self.probe.is_some() {
            return;
        }
        self.restored = true;
        let Some(store) = &self.store else {
            self.emit(CoreEvent::Restored {
                saved: false,
                layout: layout::Layout::default(),
                keys: BTreeMap::new(),
            });
            return;
        };
        let records = store.agents();
        let layout = store.layout();
        let (Ok(records), Ok(layout)) = (records, layout) else {
            self.emit(CoreEvent::ConfigError(
                "Could not restore saved layout or agents".into(),
            ));
            self.emit(CoreEvent::Restored {
                saved: false,
                layout: layout::Layout::default(),
                keys: BTreeMap::new(),
            });
            return;
        };
        let saved = layout.is_some();
        let mut layout = layout.unwrap_or_default();
        for record in records {
            if !self.config.projects.iter().any(|p| p.id == record.project) {
                continue;
            }
            let id = self.agent_entry(record, agent::Request::Resume);
            self.entries
                .get_mut(&id)
                .unwrap()
                .agent
                .as_mut()
                .unwrap()
                .exited();
            self.changed(id);
        }
        let mut keys: BTreeMap<_, _> = self
            .entries
            .iter()
            .map(|(id, entry)| (self.key(entry), *id))
            .collect();
        for tab in &mut layout.tabs {
            if !matches!(tab.kind, layout::Kind::Shell) {
                continue;
            }
            if !self
                .config
                .projects
                .iter()
                .any(|project| project.id == tab.project)
            {
                continue;
            }
            let id = SessionId(self.next_id);
            self.next_id += 1;
            self.entries.insert(
                id,
                Entry {
                    info: SessionInfo {
                        agent: None,
                        process: None,
                        config_changed: false,
                        pid: None,
                        id,
                        launch: LaunchId(0),
                        project: tab.project.clone(),
                        name: tab.name.clone(),
                        status: SessionStatus::Starting,
                    },
                    cwd: tab
                        .cwd
                        .clone()
                        .filter(|path| path.is_absolute() && path.is_dir()),
                    agent: None,
                    process: None,
                    desired: None,
                    history: None,
                    joining: false,
                    session: None,
                    size: Dimensions::default(),
                    stopping: None,
                    close: false,
                    restart: false,
                    drained: false,
                    exited: false,
                },
            );
            keys.insert(tab.key.clone(), id);
            self.spawn(id);
            // Layout keys in this event retain their old values; the UI maps
            // selection to the newly allocated IDs, then saves fresh keys.
        }
        self.emit(CoreEvent::Restored {
            saved,
            layout,
            keys,
        });
    }
    fn owner(&self, id: SessionId) -> String {
        let entry = &self.entries[&id];
        let info = &entry.info;
        if let Some(runtime) = &entry.agent {
            return format!("agent:{}", runtime.info.record.id);
        }
        info.process.as_ref().map_or_else(
            || format!("shell:{}:{}", info.project, id.0),
            |process| format!("process:{}/{}", info.project, process),
        )
    }
    fn sync_processes(&mut self) {
        let configured: Vec<_> = self
            .config
            .projects
            .iter()
            .flat_map(|project| {
                project
                    .processes
                    .iter()
                    .map(|spec| (project.id.clone(), spec.clone()))
            })
            .collect();
        for entry in self.entries.values_mut() {
            if let Some(runtime) = &entry.process {
                entry.desired = configured
                    .iter()
                    .find(|(project, spec)| {
                        project == &entry.info.project
                            && Some(&spec.id) == entry.info.process.as_ref()
                    })
                    .map(|(_, spec)| spec.clone());
                entry.info.config_changed = entry.desired.as_ref() != Some(&runtime.spec);
                if let Some(desired) = &entry.desired {
                    entry.info.name = desired.name.clone();
                }
            }
        }
        for (project, spec) in configured {
            if self.entries.values().any(|entry| {
                entry.info.project == project && entry.info.process.as_ref() == Some(&spec.id)
            }) {
                continue;
            }
            let id = SessionId(self.next_id);
            self.next_id += 1;
            self.entries.insert(
                id,
                Entry {
                    agent: None,
                    cwd: None,
                    info: SessionInfo {
                        agent: None,
                        id,
                        launch: LaunchId(0),
                        pid: None,
                        project,
                        name: spec.name.clone(),
                        process: Some(spec.id.clone()),
                        config_changed: false,
                        status: SessionStatus::Stopped,
                    },
                    process: Some(process::Runtime::new(spec.clone())),
                    desired: Some(spec),
                    history: None,
                    joining: false,
                    session: None,
                    size: Dimensions::default(),
                    stopping: None,
                    close: false,
                    restart: false,
                    drained: true,
                    exited: true,
                },
            );
        }
        for id in self.entries.keys().copied().collect::<Vec<_>>() {
            if self.entries[&id].process.is_some() {
                self.changed(id);
            }
        }
    }
    fn start_process(&mut self, id: SessionId, manual: bool) {
        if self.recovery_pending
            || self.orphan_check_at.is_some()
            || self.shutdown_deadline.is_some()
        {
            self.emit(CoreEvent::ConfigError(
                "Resolve previous sessions before starting processes".into(),
            ));
            return;
        }
        let owner = if self.entries.contains_key(&id) {
            self.owner(id)
        } else {
            return;
        };
        if self.orphans.iter().any(|orphan| {
            orphan.owner == owner
                && !orphan
                    .session
                    .members()
                    .is_ok_and(|members| members.is_empty())
        }) {
            self.emit(CoreEvent::ConfigError("Previous instance is still running; use Recover Previous Sessions to stop it first".into()));
            if let Some(runtime) = self
                .entries
                .get_mut(&id)
                .and_then(|entry| entry.process.as_mut())
            {
                runtime.retry_at = None;
            }
            return;
        }
        let Some(entry) = self.entries.get_mut(&id) else {
            return;
        };
        if entry.session.is_some() || entry.joining || entry.stopping.is_some() || !entry.exited {
            return;
        }
        if entry.desired.is_none() {
            if let Some(runtime) = &mut entry.process {
                runtime.retry_at = None;
            }
            self.emit(CoreEvent::ConfigError(
                "Process is no longer configured".into(),
            ));
            return;
        }
        let Some(runtime) = entry.process.as_mut() else {
            return;
        };
        runtime.start(manual);
        self.spawn(id);
    }
    fn autostart(&mut self, project: &str) {
        let ids: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.info.project == project
                    && entry.desired.as_ref().is_some_and(|spec| spec.autostart)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.start_process(id, true);
        }
    }
    fn complete_stop(&mut self, id: SessionId) {
        let Some(entry) = self.entries.get_mut(&id) else {
            return;
        };
        entry.stopping = None;
        entry.info.pid = None;
        if entry.agent.is_some() && matches!(entry.info.status, SessionStatus::Stopping) {
            entry.info.status = SessionStatus::Stopped;
        }
        if entry.close {
            self.removing.remove(&id);
            self.entries.remove(&id);
            self.emit(CoreEvent::SessionClosed(id));
        } else if entry.restart {
            if let Some(runtime) = &mut entry.process {
                runtime.start(true);
            }
            self.spawn(id);
        } else {
            self.changed(id);
        }
        if self.shutdown_deadline.is_some() {
            self.emit(CoreEvent::ShutdownProgress {
                remaining: self.entries.len(),
            });
        }
    }
    fn capture_codex_orphan_jobs(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        let agents = match store.agents() {
            Ok(agents) => agents,
            Err(error) => {
                self.emit(CoreEvent::ConfigError(error));
                return;
            }
        };
        let owners: BTreeSet<_> = agents
            .iter()
            .filter(|agent| agent.provider == agent::Provider::Codex)
            .map(|agent| format!("agent:{}", agent.id))
            .collect();
        let mut extra = vec![];
        for orphan in &self.orphans {
            if !owners.contains(&orphan.owner) {
                continue;
            }
            let Ok(sessions) = orphan.session.descendant_sessions() else {
                continue;
            };
            for session in sessions {
                if self
                    .orphans
                    .iter()
                    .chain(extra.iter())
                    .any(|old: &Orphan| old.session.leader == session.leader)
                {
                    continue;
                }
                if let Err(error) = store.insert(session, 0, &orphan.owner) {
                    self.emit(CoreEvent::ConfigError(error));
                    return;
                }
                let members = session.members().map_or(0, |members| members.len());
                extra.push(Orphan {
                    session,
                    owner: orphan.owner.clone(),
                    members,
                });
            }
        }
        self.orphans.extend(extra);
    }
    fn recover_orphans(&mut self, now: Duration) {
        if !self.orphan_check_at.is_some_and(|due| now >= due) {
            return;
        }
        self.orphan_check_at = Some(now + Duration::from_millis(100));
        let kill = self.orphan_kill_at.is_some_and(|due| now >= due);
        let mut remaining = vec![];
        for orphan in self.orphans.drain(..) {
            match orphan.session.members() {
                Ok(members) if members.is_empty() => {
                    if let Some(store) = &self.store {
                        let _ = store.remove(orphan.session);
                    }
                }
                Ok(members) => {
                    if kill {
                        for member in members {
                            let _ = member.signal(libc::SIGKILL, orphan.session.leader.pid);
                        }
                    }
                    remaining.push(orphan);
                }
                Err(_) => remaining.push(orphan),
            }
        }
        self.orphans = remaining;
        if self.orphans.is_empty() {
            self.orphan_check_at = None;
            self.orphan_kill_at = None;
            for project in self.opened.clone() {
                self.autostart(&project);
            }
        }
    }
    fn timers(&mut self) {
        self.decay_agents();
        let now = self.clock.now();
        let expired: Vec<_> = self
            .pending_prompt_deletes
            .iter()
            .filter(|(_, (_, due))| *due <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some((request, _)) = self.pending_prompt_deletes.remove(&id) {
                let _ = request.reply.send(mcp::Reply {
                    result: None,
                    error: Some("Prompt deletion confirmation timed out".into()),
                });
            }
        }
        self.scan_codex_jobs(now);
        if self.layout_due.is_some_and(|deadline| now >= deadline) {
            self.flush_layout();
        }
        if self.config_due.is_some_and(|deadline| now >= deadline) {
            self.config_due = None;
            self.load_config();
        }
        self.recover_orphans(now);
        let retries: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry
                    .process
                    .as_ref()
                    .is_some_and(|process| process.retry_due(now))
                    && !self.recovery_pending
                    && self.orphan_check_at.is_none()
                    && self.shutdown_deadline.is_none()
                    && entry.session.is_none()
                    && !entry.joining
                    && entry.stopping.is_none()
            })
            .map(|(id, _)| *id)
            .collect();
        for id in retries {
            self.start_process(id, false);
        }
        let ids: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .stopping
                    .as_ref()
                    .filter(|stop| now >= stop.next_check)
                    .map(|_| *id)
            })
            .collect();
        for id in ids {
            self.track_codex_descendants(id);
            let entry = self.entries.get_mut(&id).expect("scheduled session");
            let stop = entry.stopping.as_mut().expect("scheduled stop");
            stop.next_check = now + Duration::from_millis(100);
            if entry.joining {
                continue;
            }
            let empty = if let Some(session) = &entry.session {
                if let Some(signal) = stop.due_signal(now) {
                    let _ = session.signal(signal);
                }
                session.sessions_empty().is_ok_and(|empty| empty) && entry.drained
            } else {
                entry.exited
            };
            if !empty {
                continue;
            }
            if let Some(session) = entry.session.take() {
                let owned = session.owned_sessions();
                let launch = entry.info.launch;
                entry.joining = true;
                if let Some(store) = &self.store {
                    for session in owned {
                        if let Err(error) = store.remove(session) {
                            self.emit(CoreEvent::ConfigError(error));
                        }
                    }
                }
                let sender = self.sender.clone();
                // Joining may perform OS work; keep the actor responsive too.
                std::thread::spawn(move || {
                    let result = session.finish().map_err(|error| error.to_string());
                    let _ = sender.send(Message::Reaped { id, launch, result });
                });
            } else {
                if let Some(process) = &mut entry.process {
                    process.cleaned(now);
                }
                self.complete_stop(id);
            }
        }
    }
}

// Opt-in M1/M2 measurement programs share the production spawn/lifecycle path.
fn probe_command(
    probe: Option<&str>,
    environment: &ShellEnvironment,
) -> Result<(String, Vec<String>), String> {
    Ok(match probe {
        Some("scrollback") => ("/usr/bin/python3".into(), vec!["-u".into(), "-c".into(), "import os,signal; data=(b'x'*99+b'\\r\\n' )*5300;\nwhile data:\n n=os.write(1,data); data=data[n:]\nwhile True: signal.pause()".into()]),
        Some("yes") => ("/usr/bin/yes".into(), vec![]),
        Some("echo") => ("/usr/bin/python3".into(), vec!["-u".into(), "-c".into(), "import os,tty; tty.setraw(0); os.write(1,b'READY\\r\\n');\nwhile True:\n data=os.read(0,4096)\n if not data: break\n os.write(1,data)".into()]),
        Some("flood") => ("/bin/zsh".into(), vec!["-c".into(), "cat /private/tmp/devdesk-perf-flood.txt; printf '\\nFLOOD_DONE\\n'; read -r line; printf 'ECHO:%s\\n' \"$line\"; sleep 5".into()]),
        Some("seq") => ("/bin/zsh".into(), vec!["-c".into(), "seq 1 2000000; printf '\\nSEQ_DONE\\n'; sleep 5".into()]),
        Some(agent @ ("claude" | "codex")) => {
            let path = environment.variables.get("PATH").into_iter().flat_map(|path| std::env::split_paths(path)).map(|dir| dir.join(agent)).find(|path| path.is_file()).ok_or_else(|| format!("{agent} not on PATH"))?;
            (path.to_string_lossy().into_owned(), if agent == "codex" { vec!["--no-daemon".into()] } else { vec![] })
        }
        _ => (environment.shell.clone(), vec!["-l".into()]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeClock(Duration);
    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            self.0
        }
    }
    #[test]
    fn mcp_library_calls_require_live_launch_revision_and_primary_role_for_writes() {
        let (sender, _) = mpsc::channel();
        let (events, receiver) = async_channel::unbounded();
        let mut actor = Actor::new(PathBuf::new(), None, sender, events);
        actor.store = Some(Store::open(std::path::Path::new(":memory:")).unwrap());
        let record = agent::Record {
            id: uuid::Uuid::new_v4().to_string(),
            project: "p".into(),
            name: "Agent".into(),
            cwd: PathBuf::from("/tmp"),
            role: agent::Role::Primary,
            launch_args: vec![],
            provider: agent::Provider::Claude,
            session_id: None,
            parent: None,
            snapshot_at: None,
        };
        let id = actor.agent_entry(record.clone(), agent::Request::New);
        let entry = actor.entries.get_mut(&id).unwrap();
        entry.info.launch = LaunchId(7);
        entry.info.status = SessionStatus::Running;
        entry.exited = false;
        let token = entry.agent.as_ref().unwrap().token.clone();
        let operation = mcp::Operation::Write {
            write: prompts::Write {
                id: None,
                title: "Design".into(),
                body: "Polished and minimal.".into(),
                expected_revision: None,
            },
        };
        let call = |actor: &mut Actor, launch, operation| {
            let (reply, receive) = mpsc::sync_channel(1);
            actor.mcp_request(mcp::Request {
                envelope: mcp::Envelope {
                    agent_id: record.id.clone(),
                    launch,
                    token: token.clone(),
                    operation,
                },
                reply,
            });
            receive.try_recv().unwrap()
        };
        assert!(call(&mut actor, 6, operation.clone()).error.is_some());
        let created = call(&mut actor, 7, operation.clone());
        assert!(created.error.is_none());
        let prompt: prompts::Prompt = serde_json::from_value(created.result.unwrap()).unwrap();
        assert_eq!(
            actor
                .store
                .as_ref()
                .unwrap()
                .read_prompt(&prompt.id)
                .unwrap()
                .unwrap()
                .body,
            "Polished and minimal."
        );
        let mut meeting = meetings::Meeting::new(PathBuf::from("/tmp/tervy-mcp-meeting"));
        meeting.status = meetings::Status::Ready;
        meeting.title = "Planning".into();
        meeting.notes = "## Summary\n\nOriginal notes.".into();
        meeting.transcript = "Private transcript".into();
        actor
            .store
            .as_ref()
            .unwrap()
            .save_meeting(&mut meeting)
            .unwrap();
        let listed = call(
            &mut actor,
            7,
            mcp::Operation::ListMeetings {
                query: "original".into(),
                limit: 10,
                offset: 0,
            },
        );
        let listed = listed.result.unwrap();
        assert_eq!(listed[0]["id"], meeting.id);
        assert!(listed[0].get("notes").is_none());
        let read = call(
            &mut actor,
            7,
            mcp::Operation::ReadMeeting {
                id: meeting.id.clone(),
            },
        )
        .result
        .unwrap();
        assert_eq!(read["notes"], meeting.notes);
        assert!(read.get("transcript").is_none());
        assert!(read.get("audio_path").is_none());
        let edit = mcp::Operation::EditMeeting {
            edit: mcp::MeetingEdit {
                id: meeting.id.clone(),
                expected_revision: meeting.revision,
                title: Some("Planning review".into()),
                notes: Some("## Decisions\n\nApproved.".into()),
            },
        };
        assert!(call(&mut actor, 6, edit.clone()).error.is_some());
        let edited = call(&mut actor, 7, edit.clone());
        assert!(edited.error.is_none());
        assert_eq!(edited.result.unwrap()["revision"], 2);
        let saved_meeting = actor
            .store
            .as_ref()
            .unwrap()
            .meeting(&meeting.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved_meeting.title, "Planning review");
        assert_eq!(saved_meeting.notes, "## Decisions\n\nApproved.");
        assert!(call(&mut actor, 7, edit).error.unwrap().contains("changed"));
        actor
            .entries
            .get_mut(&id)
            .unwrap()
            .agent
            .as_mut()
            .unwrap()
            .info
            .record
            .role = agent::Role::Side;
        assert!(
            call(&mut actor, 7, operation)
                .error
                .unwrap()
                .contains("Read-only")
        );
        assert!(
            call(&mut actor, 7, mcp::Operation::Read { id: prompt.id })
                .error
                .is_none()
        );
        assert!(
            call(
                &mut actor,
                7,
                mcp::Operation::EditMeeting {
                    edit: mcp::MeetingEdit {
                        id: meeting.id.clone(),
                        expected_revision: saved_meeting.revision,
                        title: None,
                        notes: Some("Should fail".into()),
                    }
                }
            )
            .error
            .unwrap()
            .contains("Read-only")
        );
        assert!(
            call(
                &mut actor,
                7,
                mcp::Operation::ReadMeeting { id: meeting.id }
            )
            .error
            .is_none()
        );
        let mut saw_saved = false;
        let mut saw_meeting = false;
        while let Ok(event) = receiver.try_recv() {
            saw_saved |= matches!(&event, CoreEvent::PromptSaved { from_ui: false, .. });
            saw_meeting |= matches!(&event, CoreEvent::MeetingUpdated(_));
        }
        assert!(saw_saved);
        assert!(saw_meeting);
    }
    #[test]
    fn a_side_cannot_adopt_its_parent_id_or_a_nonfork_session_start() {
        let parent = uuid::Uuid::new_v4().to_string();
        for (source, session_id) in [
            ("fork", Some(parent.clone())),
            ("resume", Some(uuid::Uuid::new_v4().to_string())),
            ("fork", None),
        ] {
            let (sender, _) = mpsc::channel();
            let (events, receiver) = async_channel::unbounded();
            let mut actor = Actor::new(PathBuf::new(), None, sender, events);
            let record = agent::Record {
                id: uuid::Uuid::new_v4().to_string(),
                project: "p".into(),
                name: "Side".into(),
                cwd: PathBuf::from("/tmp"),
                role: agent::Role::Side,
                launch_args: vec![],
                provider: crate::agent::Provider::Claude,
                session_id: None,
                parent: Some(uuid::Uuid::new_v4().to_string()),
                snapshot_at: Some(1),
            };
            let id = actor.agent_entry(
                record.clone(),
                agent::Request::Fork {
                    parent_session_id: parent.clone(),
                    question: "Why?".into(),
                },
            );
            let entry = actor.entries.get_mut(&id).unwrap();
            entry.info.launch = LaunchId(9);
            entry.info.status = SessionStatus::Starting;
            entry.exited = false;
            let token = entry.agent.as_ref().unwrap().token.clone();
            actor.hook(ipc::HookEvent {
                agent_id: record.id,
                launch: 9,
                token,
                event: "SessionStart".into(),
                session_id,
                source: Some(source.into()),
                ..Default::default()
            });
            assert!(
                actor.entries[&id]
                    .agent
                    .as_ref()
                    .unwrap()
                    .info
                    .record
                    .session_id
                    .is_none()
            );
            assert!(
                std::iter::from_fn(|| receiver.try_recv().ok())
                    .any(|event| matches!(event, CoreEvent::ConfigError(_)))
            );
        }
    }
    #[test]
    fn stale_or_unauthenticated_hooks_cannot_update_agent_and_rate_is_bounded() {
        let (sender, _) = mpsc::channel();
        let (events, receiver) = async_channel::unbounded();
        let mut actor = Actor::new(PathBuf::new(), None, sender, events);
        let record = agent::Record {
            id: uuid::Uuid::new_v4().to_string(),
            project: "p".into(),
            name: "Claude".into(),
            cwd: PathBuf::from("/tmp"),
            role: agent::Role::Primary,
            launch_args: vec![],
            provider: crate::agent::Provider::Claude,
            parent: None,
            snapshot_at: None,
            session_id: Some(uuid::Uuid::new_v4().to_string()),
        };
        let id = actor.agent_entry(record.clone(), agent::Request::New);
        let entry = actor.entries.get_mut(&id).unwrap();
        entry.info.launch = LaunchId(9);
        entry.exited = false;
        let token = entry.agent.as_ref().unwrap().token.clone();
        let event = ipc::HookEvent {
            agent_id: record.id,
            launch: 9,
            token,
            event: "UserPromptSubmit".into(),
            ..Default::default()
        };
        actor.hook(ipc::HookEvent {
            launch: 8,
            ..event.clone()
        });
        actor.hook(ipc::HookEvent {
            token: "wrong".into(),
            ..event.clone()
        });
        assert!(receiver.is_empty());
        actor.hook(event.clone());
        assert_eq!(
            actor.entries[&id].agent.as_ref().unwrap().info.status,
            agent::Status::Working
        );
        while receiver.try_recv().is_ok() {}
        for _ in 0..150 {
            actor.hook(event.clone());
        }
        let mut count = 0;
        while receiver.try_recv().is_ok() {
            count += 1;
        }
        assert!(count <= 99);
        // A late SessionStart must not recreate an agent deleted while stopping.
        let folder =
            std::env::temp_dir().join(format!("devdesk-removed-hook-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let store = Store::open(&folder.join("state.db")).unwrap();
        store
            .save_agent(&actor.entries[&id].agent.as_ref().unwrap().info.record)
            .unwrap();
        actor.store = Some(store);
        actor.clock = Box::new(FakeClock(Duration::from_secs(2)));
        actor.remove_session(id);
        actor.hook(ipc::HookEvent {
            event: "SessionStart".into(),
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            ..event.clone()
        });
        assert!(actor.store.as_ref().unwrap().agents().unwrap().is_empty());
        actor
            .entries
            .get_mut(&id)
            .unwrap()
            .agent
            .as_mut()
            .unwrap()
            .exited();
        actor.entries.get_mut(&id).unwrap().exited = true;
        actor.hook(ipc::HookEvent {
            event: "PermissionRequest".into(),
            ..event
        });
        assert_eq!(
            actor.entries[&id].agent.as_ref().unwrap().info.status,
            agent::Status::Exited
        );
        drop(actor);
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn recovery_pauses_due_retries_without_dropping_them() {
        let (sender, _) = mpsc::channel();
        let (events, receiver) = async_channel::unbounded();
        let mut actor = Actor::new(PathBuf::new(), None, sender, events);
        actor.config = config::parse(
            "[[projects]]\nid='p'\nname='P'\npath='/tmp'\n[[projects.processes]]\nid='worker'\nname='Worker'\ncommand='exit 1'\nrestart={policy='on-failure',max_retries=5}",
            &PathBuf::from("/tmp"),
        ).unwrap();
        actor.sync_processes();
        while receiver.try_recv().is_ok() {}
        let process = actor
            .entries
            .get_mut(&SessionId(1))
            .unwrap()
            .process
            .as_mut()
            .unwrap();
        process.start(true);
        process.spawn_failed("failed".into(), Duration::ZERO);
        actor.clock = Box::new(FakeClock(Duration::from_secs(2)));
        actor.recovery_pending = true;
        actor.timers();
        assert!(receiver.is_empty());
        assert_eq!(
            actor.entries[&SessionId(1)]
                .process
                .as_ref()
                .unwrap()
                .retry_at,
            Some(Duration::from_secs(1))
        );
    }
    #[test]
    fn configured_stop_respects_grace_and_three_second_escalation() {
        let config = config::parse(
            "[[projects]]\nid='p'\nname='P'\npath='/tmp'\n[[projects.processes]]\nid='p'\nname='P'\ncommand='sleep 10'\nstop_timeout_secs=1",
            &PathBuf::from("/tmp"),
        ).unwrap();
        let mut clock = FakeClock(Duration::from_secs(10));
        let mut stop = StopPlan::for_process(clock.now(), &config.projects[0].processes[0]);
        assert_eq!(stop.due_signal(clock.now()), Some(libc::SIGINT));
        clock.0 += Duration::from_millis(999);
        assert_eq!(stop.due_signal(clock.now()), None);
        clock.0 += Duration::from_millis(1);
        assert_eq!(stop.due_signal(clock.now()), Some(libc::SIGTERM));
        clock.0 += Duration::from_millis(2999);
        assert_eq!(stop.due_signal(clock.now()), None);
        clock.0 += Duration::from_millis(1);
        assert_eq!(stop.due_signal(clock.now()), Some(libc::SIGKILL));
    }
    #[test]
    fn escalation_uses_deadlines_and_stale_lifecycle_is_ignored() {
        let mut clock = FakeClock(Duration::ZERO);
        let mut stop = StopPlan::new(clock.now());
        assert_eq!(stop.due_signal(clock.now()), Some(libc::SIGHUP));
        clock.0 = Duration::from_secs(1);
        assert_eq!(stop.due_signal(clock.now()), None);
        clock.0 = Duration::from_secs(2);
        assert_eq!(stop.due_signal(clock.now()), Some(libc::SIGTERM));
        clock.0 = Duration::from_secs(5);
        assert_eq!(stop.due_signal(clock.now()), Some(libc::SIGKILL));
        let (sender, _) = mpsc::channel();
        let (events, receiver) = async_channel::unbounded();
        let mut actor = Actor::new(PathBuf::new(), None, sender, events);
        actor.entries.insert(
            SessionId(1),
            Entry {
                agent: None,
                cwd: None,
                process: None,
                desired: None,
                history: None,
                joining: false,
                info: SessionInfo {
                    agent: None,
                    process: None,
                    config_changed: false,
                    pid: None,
                    id: SessionId(1),
                    launch: LaunchId(2),
                    project: "a".into(),
                    name: "Shell".into(),
                    status: SessionStatus::Running,
                },
                session: None,
                size: Dimensions::default(),
                stopping: None,
                close: false,
                restart: false,
                drained: false,
                exited: false,
            },
        );
        actor.message(Message::Lifecycle {
            id: SessionId(1),
            launch: LaunchId(1),
            signal: TerminalSignal::Exit,
        });
        assert!(!actor.entries[&SessionId(1)].drained);
        assert!(receiver.is_empty());
        actor.stop(SessionId(1), true, false);
        assert!(matches!(
            actor.entries[&SessionId(1)].info.status,
            SessionStatus::Stopping
        ));
        assert!(actor.entries[&SessionId(1)].close);
    }
    #[test]
    fn codex_title_after_notify_persists_identity_and_updates_the_ui() {
        let (sender, _) = mpsc::channel();
        let (events, receiver) = async_channel::unbounded();
        let mut actor = Actor::new(PathBuf::new(), None, sender, events);
        actor.store = Some(Store::open(std::path::Path::new(":memory:")).unwrap());
        let record = agent::Record {
            id: uuid::Uuid::new_v4().to_string(),
            project: "p".into(),
            name: "Codex".into(),
            cwd: PathBuf::from("/tmp"),
            role: agent::Role::Primary,
            launch_args: vec![],
            provider: agent::Provider::Codex,
            parent: None,
            snapshot_at: None,
            session_id: None,
        };
        let id = actor.agent_entry(record.clone(), agent::Request::New);
        let entry = actor.entries.get_mut(&id).unwrap();
        entry.info.launch = LaunchId(9);
        entry.info.status = SessionStatus::Running;
        entry.exited = false;
        let token = entry.agent.as_ref().unwrap().token.clone();
        let thread = "20000000-0000-4000-8000-000000000002";
        actor.hook(ipc::HookEvent {
            agent_id: record.id,
            launch: 9,
            token,
            event: "agent-turn-complete".into(),
            session_id: Some(thread.into()),
            ..Default::default()
        });
        while receiver.try_recv().is_ok() {}
        let title = "20000000-0000-4000-8000-0000000...";
        actor.command(Command::AgentTitle {
            id,
            launch: LaunchId(8),
            title: title.into(),
        });
        assert!(receiver.is_empty());
        assert!(actor.store.as_ref().unwrap().agents().unwrap().is_empty());
        actor.command(Command::AgentTitle {
            id,
            launch: LaunchId(9),
            title: title.into(),
        });
        let saved = actor.store.as_ref().unwrap().agents().unwrap();
        assert_eq!(saved[0].session_id.as_deref(), Some(thread));
        let CoreEvent::SessionChanged(info) = receiver.try_recv().unwrap() else {
            panic!("missing UI update")
        };
        assert_eq!(info.status, SessionStatus::Running);
        let agent = info.agent.unwrap();
        assert_eq!(agent.status, agent::Status::Idle);
        assert_eq!(agent.record.session_id.as_deref(), Some(thread));
    }
}
