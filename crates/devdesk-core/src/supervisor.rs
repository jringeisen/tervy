//! One actor owns every PTY. Views hold only cheap terminal handles and send
//! commands; no view destructor kills a process. Lifecycle callbacks use the
//! actor mailbox, while output/wakeups go directly to the terminal view.
use crate::config::{self, Config};
use crate::env::ShellEnvironment;
use devdesk_term::{
    Dimensions, SpawnSpec, TerminalEvents, TerminalHandle, TerminalSession, TerminalSignal,
};
use notify::Watcher;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchId(pub u64);
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Starting,
    Running,
    Stopping,
    Exited { code: Option<i32> },
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub pid: Option<u32>,
    pub id: SessionId,
    pub launch: LaunchId,
    pub project: String,
    pub name: String,
    pub status: SessionStatus,
}

pub enum Command {
    NewShell {
        project: String,
        size: Dimensions,
    },
    Close {
        id: SessionId,
        confirmed: Option<LaunchId>,
    },
    Restart(SessionId),
    Rename {
        id: SessionId,
        name: String,
    },
    AddProject(PathBuf),
    ReloadConfig,
    Shutdown,
}
pub enum CoreEvent {
    Config(Config),
    ConfigError(String),
    ProjectAdded(String),
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
        let (sender, receiver) = mpsc::channel();
        let (events, event_rx) = async_channel::unbounded();
        let handle = SupervisorHandle {
            sender: sender.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("devdesk-supervisor".into())
            .spawn(move || {
                Actor::new(path, probe, sender, events).run(receiver);
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
}
impl StopPlan {
    fn new(now: Duration) -> Self {
        Self {
            started: now,
            next_check: now,
            stage: 0,
        }
    }
    fn due_signal(&mut self, now: Duration) -> Option<i32> {
        let age = now.saturating_sub(self.started);
        let next = if age >= Duration::from_secs(5) {
            3
        } else if age >= Duration::from_secs(2) {
            2
        } else {
            1
        };
        if next > self.stage {
            self.stage = next;
            Some(match next {
                1 => libc::SIGHUP,
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
            self.emit(CoreEvent::SessionChanged(entry.info.clone()));
        }
    }
    fn load_config(&mut self) {
        match config::initialize(&self.path).and_then(|_| config::read(&self.path)) {
            Ok(config) => {
                self.config = config;
                self.emit(CoreEvent::Config(self.config.clone()));
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
            self.load_config();
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
                .chain(self.config_due)
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
            if self.shutdown_deadline.is_some() && self.entries.is_empty() {
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
                        let handle = session.handle.clone();
                        let entry = self.entries.get_mut(&id).expect("checked session");
                        entry.info.pid = Some(session.pid);
                        entry.session = Some(session);
                        if entry.stopping.is_none() && !entry.exited {
                            entry.info.status = SessionStatus::Running;
                        }
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
                        entry.info.status = SessionStatus::Failed(error);
                        if entry.close {
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
                            if entry.stopping.is_none() {
                                entry.info.status = SessionStatus::Exited {
                                    code: status.code(),
                                };
                            }
                            // A dead shell may leave jobs holding its PTY. Clean
                            // those up even if the terminal never reaches EOF.
                            entry
                                .stopping
                                .get_or_insert_with(|| StopPlan::new(self.clock.now()));
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
            Command::ReloadConfig => self.load_config(),
            Command::AddProject(folder) => match config::append_project(&self.path, &folder) {
                Ok((config, id)) => {
                    self.config = config;
                    self.emit(CoreEvent::Config(self.config.clone()));
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
                        info: SessionInfo {
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
                    && let Some(entry) = self.entries.get_mut(&id)
                {
                    entry.info.name = name;
                    self.changed(id);
                }
            }
            Command::Close { id, confirmed } => {
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
            Command::Restart(id) => self.stop(id, false, true),
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
    fn stop(&mut self, id: SessionId, close: bool, restart: bool) {
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.close |= close;
            entry.restart = restart && !entry.close;
            entry
                .stopping
                .get_or_insert_with(|| StopPlan::new(self.clock.now()));
            entry.info.status = SessionStatus::Stopping;
            self.changed(id);
        }
    }
    fn spawn(&mut self, id: SessionId) {
        let entry = self.entries.get_mut(&id).expect("registered before spawn");
        let Some(project) = self
            .config
            .projects
            .iter()
            .find(|project| project.id == entry.info.project)
        else {
            entry.info.status = SessionStatus::Failed("Project was removed from config".into());
            entry.exited = true;
            entry.drained = true;
            entry.stopping = None;
            self.changed(id);
            return;
        };
        let cwd = project.path.clone();
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
        if let Err(error) = std::thread::Builder::new()
            .name("devdesk-spawn".into())
            .spawn(move || {
                let environment = ShellEnvironment::shared();
                let warning = environment.warning.clone();
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
                    let mut env = environment.variables.clone();
                    env.insert("DEVDESK_SESSION_ID".into(), id.0.to_string());
                    env.insert("DEVDESK_LAUNCH_ID".into(), launch.0.to_string());
                    spawn_started = probe.as_ref().map(|_| Instant::now());
                    TerminalSession::spawn_observed(
                        SpawnSpec {
                            program,
                            args,
                            cwd,
                            env,
                            scrollback_lines: lines,
                        },
                        size,
                        Some(Box::new(move |signal| {
                            let _ = sender.send(Message::Lifecycle { id, launch, signal });
                        })),
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
    fn timers(&mut self) {
        let now = self.clock.now();
        if self.config_due.is_some_and(|deadline| now >= deadline) {
            self.config_due = None;
            self.load_config();
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
            let entry = self.entries.get_mut(&id).expect("scheduled session");
            let stop = entry.stopping.as_mut().expect("scheduled stop");
            stop.next_check = now + Duration::from_millis(100);
            let empty = if let Some(session) = &entry.session {
                if let Some(signal) = stop.due_signal(now) {
                    let _ = session.signal(signal);
                }
                session
                    .owned
                    .members()
                    .is_ok_and(|members| members.is_empty())
                    && entry.drained
            } else {
                entry.exited
            };
            if !empty {
                continue;
            }
            let close = entry.close;
            let restart = entry.restart;
            // Drop delegates reader shutdown/join to term's reaper worker.
            entry.session.take();
            entry.stopping = None;
            if close {
                self.entries.remove(&id);
                self.emit(CoreEvent::SessionClosed(id));
            } else if restart {
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
                info: SessionInfo {
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
