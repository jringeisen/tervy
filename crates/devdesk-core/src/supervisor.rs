//! One actor owns every PTY. Views hold only cheap terminal handles and send
//! commands; no view destructor kills a process. Lifecycle callbacks use the
//! actor mailbox, while output/wakeups go directly to the terminal view.
use crate::config::{self, Config};
use crate::env::ShellEnvironment;
use crate::{
    agent::{self, AgentProvider},
    ipc, layout,
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
    NewAgent {
        project: String,
        picker: bool,
        provider: agent::Provider,
    },
    ResumeAgent(SessionId),
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
    ReloadConfig,
    Shutdown,
}
pub enum CoreEvent {
    Restored {
        saved: bool,
        layout: layout::Layout,
        keys: BTreeMap<String, SessionId>,
    },
    Orphans(Vec<Orphan>),
    Config(Config),
    ConfigError(String),
    ProjectAdded(String),
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
                .chain(self.orphan_check_at)
                .chain(self.config_due)
                .chain(self.layout_due)
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
            if self.shutdown_deadline.is_some()
                && self.entries.is_empty()
                && self.orphan_check_at.is_none()
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
                        entry.info.agent = entry.agent.as_ref().map(|runtime| runtime.info.clone());
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
    fn command(&mut self, command: Command) {
        if self.shutdown_deadline.is_some() && !matches!(command, Command::Shutdown) {
            return;
        }
        match command {
            Command::NewAgent {
                project,
                picker,
                provider,
            } => self.new_agent(project, picker, provider),
            Command::AgentInput { id, launch, enter } => self.agent_input(id, launch, Some(enter)),
            Command::AgentBell { id, launch } => self.agent_input(id, launch, None),
            Command::AgentTitle { id, launch, title } => {
                if let Some(entry) = self.entries.get_mut(&id)
                    && entry.info.launch == launch
                    && let Some(runtime) = &mut entry.agent
                    && runtime.info.record.provider == agent::Provider::Codex
                {
                    runtime.observe_title(&title);
                }
            }
            Command::AgentExitSummary {
                id,
                launch,
                session_id,
            } => self.agent_exit_summary(id, launch, session_id),
            Command::ForkAgent { parent, question } => self.fork_agent(parent, question),
            Command::ResumeAgent(id) => self.resume_agent(id),
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
            Command::NewShell { project, size } => {
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
                            id,
                            launch: LaunchId(0),
                            project,
                            name: format!("Terminal {}", id.0),
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
                    self.resume_agent(id);
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
        for owned in &discovered {
            if let Some(store) = &self.store
                && let Err(failure) = store.insert(*owned, entry.info.launch.0, &owner)
            {
                error = Some(failure);
            }
        }
        session.track_sessions(discovered);
        if let Some(error) = error {
            self.emit(CoreEvent::ConfigError(error));
        }
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
                            Some(if id.0 == 11 { "echo" } else { "yes" })
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
    fn new_agent(&mut self, project: String, picker: bool, provider: agent::Provider) {
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
        let record = agent::Record {
            id: uuid::Uuid::new_v4().to_string(),
            project: project.id.clone(),
            name: if picker {
                format!("{} · choose session", provider.label())
            } else {
                provider.label().into()
            },
            cwd: project.path.clone(),
            role: agent::Role::Primary,
            provider,
            parent: None,
            snapshot_at: None,
            session_id: (!picker && provider == agent::Provider::Claude)
                .then(|| uuid::Uuid::new_v4().to_string()),
        };
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
            if picker {
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
    fn resume_agent(&mut self, id: SessionId) {
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
        runtime.request = if runtime.info.record.session_id.is_some() {
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
        let record =
            (previous != runtime.info.record.session_id).then(|| runtime.info.record.clone());
        if let (Some(store), Some(record)) = (&self.store, record)
            && let Err(error) = store.save_agent(&record)
        {
            self.emit(CoreEvent::ConfigError(error));
        }
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
}
