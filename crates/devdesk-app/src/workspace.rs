//! UI mirror of the supervisor. All terminal entities remain alive across tab
//! and project switches; only selected entities are attached to the render tree.
mod side_smoke;
use crate::terminal_view::TerminalView;
use devdesk_core::config::{Config, Project};
use devdesk_core::supervisor::{
    Command as CoreCommand, CoreEvent, SessionId, SessionInfo, SessionStatus, SupervisorHandle,
};
use devdesk_core::{agent, layout};
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::{BTreeMap, BTreeSet};

gpui_kit::actions!(
    devdesk,
    [
        Palette,
        NewTerminal,
        CloseTerminal,
        RenameTerminal,
        AskAgentQuestion,
        AddProject,
        NextTab,
        PreviousTab,
        Tab1,
        Tab2,
        Tab3,
        Tab4,
        Tab5,
        Tab6,
        Tab7,
        Tab8,
        Tab9
    ]
);

struct ShellTab {
    info: SessionInfo,
    view: Entity<TerminalView>,
    /// A dismissed process tab retains its view/history and sidebar identity.
    /// Shell tabs are removed by the supervisor after their session stops.
    open: bool,
}
#[derive(Clone)]
enum RemovalTarget {
    Project(String),
    Session(SessionId),
}
#[derive(Clone)]
enum PaletteAction {
    Project(String),
    New,
    Close,
    Rename,
    Add,
    Start(SessionId),
    Stop(SessionId),
    Restart(SessionId),
    StartAll(String),
    StopAll(String),
    Recover,
    LaunchAgent(agent::Provider, bool),
    ResumeAgent(SessionId),
    Ask,
    Remove(RemovalTarget),
    Select(SessionId),
    FocusPrimary,
    FocusSide,
    Reload,
    Quit,
    ToggleProject(String),
    ResizeSide(i32),
    OpenUrl(String),
}

pub struct Workspace {
    focus: FocusHandle,
    restoring: bool,
    last_layout: Option<layout::Layout>,
    approvals: usize,
    stress: Option<crate::m2_perf::ManyProbe>,
    core: SupervisorHandle,
    config: Config,
    projects: Vec<Project>,
    tabs: BTreeMap<SessionId, ShellTab>,
    // Session IDs change on restore; presentation order follows saved identities.
    tab_order: Vec<SessionId>,
    active_project: Option<String>,
    selected: BTreeMap<String, SessionId>,
    expanded: BTreeSet<String>,
    pending: BTreeSet<String>,
    ready: bool,
    perf_layout: String,
    error: Option<String>,
    palette: Entity<CommandState>,
    palette_open: bool,
    palette_query: String,
    rename: Option<SessionId>,
    question_parent: Option<SessionId>,
    /// Only the UI owns split presentation; each PTY remains actor-owned.
    split_states: BTreeMap<SessionId, Entity<ResizableState>>,
    side_panes: BTreeMap<SessionId, SessionId>,
    side_focus: Option<SessionId>,
    rename_input: Entity<InputState>,
    prompt_pending: bool,
    shutdown: Option<usize>,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}
impl Workspace {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        core: SupervisorHandle,
        events: async_channel::Receiver<CoreEvent>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(&rename_input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_rename(window, cx);
            }
        });
        let handle = window.window_handle();
        let task = cx.spawn(async move |view, cx| {
            while let Ok(event) = events.recv().await {
                if handle
                    .update(cx, |_, window, cx| {
                        let _ = view.update(cx, |this, cx| this.event(event, window, cx));
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            if std::env::var("DEVDESK_PERF_EXIT").as_deref() == Ok("1") {
                return;
            }
            let _ = weak.update(cx, |this, cx| {
                this.ready = true;
                if this.perf_layout == "side" {
                    this.smoke_side(window, cx);
                    return;
                }
                if matches!(this.perf_layout.as_str(), "idle20" | "scrollback20") {
                    this.active_project = Some("probe".into());
                    for _ in 0..20 {
                        this.core.send(CoreCommand::NewShell {
                            project: "probe".into(),
                            size: devdesk_term::Dimensions::default(),
                        });
                    }
                    let timer = cx
                        .background_executor()
                        .timer(std::time::Duration::from_secs(5));
                    this._tasks.push(cx.spawn(async move |view, cx| {
                        timer.await;
                        let _ = view.update(cx, |this, cx| {
                            use devdesk_term::GridDimensions as _;
                            let mut rows = Vec::new();
                            for tab in this.tabs.values() {
                                if let Some(session) = &tab.view.read(cx).session {
                                    let term = session.handle.term.lock();
                                    rows.push((
                                        term.columns(),
                                        term.screen_lines(),
                                        term.grid().history_size(),
                                    ));
                                }
                            }
                            eprintln!("devdesk-idle-grids {:?}", rows);
                        });
                    }));
                    return;
                }
                if this.stress.is_some() {
                    this.start_stress();
                    return;
                }
                if this.restoring {
                    this.core.send(CoreCommand::Restore);
                    return;
                }
                if !this.empty_control()
                    && let Some(project) = this.active_project.clone()
                {
                    this.open_project(project, window, cx);
                }
            });
        });
        Self {
            focus,
            restoring: std::env::var_os("DEVDESK_PERF_LAYOUT").is_none(),
            last_layout: None,
            approvals: 0,
            stress: (std::env::var("DEVDESK_PERF_LAYOUT").as_deref() == Ok("many"))
                .then(crate::m2_perf::ManyProbe::new),
            core,
            config: Config::default(),
            projects: vec![],
            tabs: BTreeMap::new(),
            tab_order: vec![],
            active_project: None,
            selected: BTreeMap::new(),
            expanded: BTreeSet::new(),
            pending: BTreeSet::new(),
            ready: false,
            perf_layout: std::env::var("DEVDESK_PERF_LAYOUT").unwrap_or_default(),
            error: None,
            palette,
            palette_open: false,
            palette_query: String::new(),
            rename: None,
            question_parent: None,
            split_states: BTreeMap::new(),
            side_panes: BTreeMap::new(),
            side_focus: None,
            rename_input,
            prompt_pending: false,
            shutdown: None,
            _tasks: vec![task],
            _subscriptions: vec![subscription],
        }
    }
    fn empty_control(&self) -> bool {
        matches!(
            self.perf_layout.as_str(),
            "normal" | "minimal" | "no-sidebar" | "terminal-ui" | "idle20" | "scrollback20"
        )
    }
    fn active(&self) -> Option<SessionId> {
        self.active_project
            .as_ref()
            .and_then(|project| self.selected.get(project))
            .copied()
    }
    fn project_tabs(&self) -> Vec<SessionId> {
        self.tab_order
            .iter()
            .copied()
            .filter(|id| {
                self.tabs.get(id).is_some_and(|tab| {
                    tab.open
                        && !is_side(&tab.info)
                        && Some(&tab.info.project) == self.active_project.as_ref()
                })
            })
            .collect()
    }
    fn event(&mut self, event: CoreEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            CoreEvent::Restored {
                saved,
                layout,
                keys,
            } => {
                if saved {
                    let mut order: Vec<_> = layout
                        .tabs
                        .iter()
                        .filter_map(|tab| keys.get(&tab.key).copied())
                        .collect();
                    for id in &self.tab_order {
                        if !order.contains(id) {
                            order.push(*id);
                        }
                    }
                    self.tab_order = order;
                    for tab in self.tabs.values_mut() {
                        tab.open = false;
                    }
                    for saved_tab in &layout.tabs {
                        if let Some(tab) = keys
                            .get(&saved_tab.key)
                            .and_then(|id| self.tabs.get_mut(id))
                        {
                            tab.open = saved_tab.open && !is_side(&tab.info);
                        }
                    }
                    self.expanded = layout.expanded.into_iter().collect();
                    self.active_project = layout
                        .active_project
                        .filter(|id| self.projects.iter().any(|project| project.id == *id));
                    self.selected.clear();
                    for key in layout.selected {
                        if let Some(id) = keys.get(&key)
                            && let Some(tab) = self.tabs.get(id)
                            && tab.open
                        {
                            self.selected.insert(tab.info.project.clone(), *id);
                        }
                    }
                }
                self.restoring = false;
                if self.active_project.is_none() {
                    self.active_project = self.projects.first().map(|project| project.id.clone());
                }
                if let Some(project) = self.active_project.clone() {
                    if saved {
                        self.core.send(CoreCommand::ProjectOpened(project.clone()));
                        if !self.selected.contains_key(&project)
                            && let Some(id) = self.project_tabs().first().copied()
                        {
                            self.selected.insert(project, id);
                        }
                        self.update_visibility(window, cx);
                    } else {
                        self.open_project(project, window, cx);
                    }
                }
            }
            CoreEvent::Orphans(orphans) => {
                let summary = orphans
                    .iter()
                    .map(|orphan| {
                        format!("{}: {} surviving processes", orphan.owner, orphan.members)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let answer = window.prompt(
                    PromptLevel::Warning,
                    "Processes from the previous DevDesk run are still running",
                    Some(&summary),
                    &["Leave running", "Stop previous processes"],
                    cx,
                );
                self._tasks.push(cx.spawn(async move |view, cx| {
                    if let Ok(answer) = answer.await {
                        let _ = view.update(cx, |this, _| {
                            this.core
                                .send(CoreCommand::ResolveOrphans { kill: answer == 1 })
                        });
                    }
                }));
            }
            CoreEvent::Config(config) => {
                self.error = None;
                let mut projects = config.projects.clone();
                // Keep removed projects reachable while they still own tabs.
                for old in &self.projects {
                    if !projects.iter().any(|project| project.id == old.id)
                        && self.tabs.values().any(|tab| tab.info.project == old.id)
                    {
                        projects.push(old.clone());
                    }
                }
                self.config = config;
                self.projects = projects;
                if self.stress.is_some() {
                    if self.ready {
                        self.start_stress();
                    }
                } else if self.active_project.is_none()
                    && !self.restoring
                    && !self.empty_control()
                    && let Some(project) = self.projects.first()
                {
                    self.open_project(project.id.clone(), window, cx);
                }
            }
            CoreEvent::ConfigError(error) => self.error = Some(error),
            CoreEvent::ProjectAdded(project) => self.open_project(project, window, cx),
            CoreEvent::ProjectRemoved(project) => {
                self.projects.retain(|p| p.id != project);
                self.expanded.remove(&project);
                self.selected.remove(&project);
                for tab in self
                    .tabs
                    .values_mut()
                    .filter(|tab| tab.info.project == project)
                {
                    tab.open = false;
                }
                if self.active_project.as_ref() == Some(&project) {
                    self.active_project = self.projects.first().map(|p| p.id.clone());
                }
                self.update_visibility(window, cx);
            }
            CoreEvent::SessionChanged(info) => {
                if self.perf_layout == "scrollback20" {
                    eprintln!(
                        "devdesk-scrollback-session id={} status={:?}",
                        info.id.0, info.status
                    );
                }

                if let Some(tab) = self.tabs.get_mut(&info.id) {
                    tab.view.update(cx, |view, cx| {
                        view.set_agent(info.agent.as_ref(), info.launch);
                        view.set_status(&info.status, cx);
                        if info.agent.is_some()
                            && matches!(
                                info.status,
                                SessionStatus::Stopped | SessionStatus::Exited { .. }
                            )
                        {
                            view.set_agent_stub(cx);
                        }
                    });
                    tab.info = info;
                } else {
                    self.pending.remove(&info.project);
                    let view = cx.new(|cx| {
                        TerminalView::new(
                            window,
                            cx,
                            info.id,
                            self.core.clone(),
                            &self.config.settings,
                        )
                    });
                    view.update(cx, |view, cx| {
                        view.set_agent(info.agent.as_ref(), info.launch);
                        view.set_status(&info.status, cx);
                        if info.agent.is_some()
                            && matches!(
                                info.status,
                                SessionStatus::Stopped | SessionStatus::Exited { .. }
                            )
                        {
                            view.set_agent_stub(cx);
                        }
                    });
                    let id = info.id;
                    let open = info.agent.is_none()
                        || !matches!(info.status, SessionStatus::Exited { .. });
                    let side = is_side(&info);
                    if info.process.is_none() && open && !side {
                        self.selected.insert(info.project.clone(), id);
                    }
                    self.tab_order.push(id);
                    self.tabs.insert(
                        id,
                        ShellTab {
                            info,
                            view,
                            open: open && !side,
                        },
                    );
                    if side && open && !self.restoring {
                        self.open_side(id);
                    }
                    self.update_visibility(window, cx);
                }
            }
            CoreEvent::SessionStarted {
                info,
                handle,
                events,
                warning,
                spawn_started,
            } => {
                if let Some(stress) = self.stress.as_mut() {
                    stress.attached += 1;
                    if let Some(pid) = info.pid {
                        stress.pids.push(pid);
                    }
                    if stress.attached == devdesk_core::perf::noisy_count() + 1 {
                        eprintln!("devdesk-many-progress attached all");
                        stress.started = Some(std::time::Instant::now());
                        self.schedule_stress(window, cx);
                    }
                }
                if let Some(tab) = self.tabs.get_mut(&info.id) {
                    tab.view.update(cx, |view, cx| {
                        view.set_agent(info.agent.as_ref(), info.launch);
                        view.attach(handle, events, warning, spawn_started, cx);
                        {
                            view.set_agent(info.agent.as_ref(), info.launch);
                            view.set_status(&info.status, cx);
                            if info.agent.is_some()
                                && matches!(
                                    info.status,
                                    SessionStatus::Stopped | SessionStatus::Exited { .. }
                                )
                            {
                                view.set_agent_stub(cx);
                            }
                        };
                    });
                    tab.info = info;
                }
            }
            CoreEvent::SessionClosed(id) => {
                self.split_states.remove(&id);
                self.side_panes
                    .retain(|parent, side| *parent != id && *side != id);
                self.tab_order.retain(|candidate| *candidate != id);
                if let Some(tab) = self.tabs.remove(&id)
                    && self.selected.get(&tab.info.project) == Some(&id)
                {
                    if let Some((&next, _)) = self.tabs.iter().find(|(_, candidate)| {
                        candidate.open && candidate.info.project == tab.info.project
                    }) {
                        self.selected.insert(tab.info.project, next);
                    } else {
                        self.selected.remove(&tab.info.project);
                    }
                }
                self.update_visibility(window, cx);
            }
            CoreEvent::ConfirmClose { id, launch, job } => {
                if self.prompt_pending {
                    return;
                }
                self.prompt_pending = true;
                let answer = window.prompt(
                    PromptLevel::Warning,
                    &format!("{job} is running"),
                    Some("Closing this terminal stops its shell and all jobs in its session."),
                    &["Cancel", "Stop and close"],
                    cx,
                );
                self._tasks.push(cx.spawn(async move |view, cx| {
                    let answer = answer.await;
                    let _ = view.update(cx, |this, _| {
                        this.prompt_pending = false;
                        if answer == Ok(1) {
                            this.core.send(CoreCommand::Close {
                                id,
                                confirmed: Some(launch),
                            });
                        }
                    });
                }));
            }
            CoreEvent::ShutdownProgress { remaining } => self.shutdown = Some(remaining),
            CoreEvent::ShutdownComplete { clean } => {
                if let Some(stress) = &self.stress {
                    stress.report(clean);
                }
                if !clean {
                    tracing::error!(
                        "shutdown deadline reached before all sessions were confirmed empty"
                    );
                }
                cx.quit();
            }
        }
        let approvals = self
            .tabs
            .values()
            .filter(|tab| {
                matches!(
                    tab.info.status,
                    SessionStatus::Starting | SessionStatus::Running
                ) && tab.info.agent.as_ref().is_some_and(|info| {
                    matches!(
                        info.status,
                        agent::Status::NeedsApproval | agent::Status::Attention
                    )
                })
            })
            .count();
        if approvals != self.approvals {
            self.approvals = approvals;
            crate::badge::set(approvals);
        }
        self.save_layout();
        cx.notify();
    }
    fn confirm_remove(
        &mut self,
        target: RemovalTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.prompt_pending {
            return;
        }
        let (name, detail) = match &target {
            RemovalTarget::Project(id) => {
                let Some(project) = self.projects.iter().find(|p| p.id == *id) else {
                    return;
                };
                (
                    project.name.clone(),
                    "Stop this project's sessions and remove it from DevDesk. Files in its folder are kept.",
                )
            }
            RemovalTarget::Session(id) => {
                let Some(tab) = self.tabs.get(id) else {
                    return;
                };
                (
                    tab.info.name.clone(),
                    if tab.info.agent.is_some() {
                        "Stop this agent and remove its DevDesk entry. The CLI's saved conversation is kept."
                    } else {
                        "Stop this process and remove its definition from DevDesk's global config."
                    },
                )
            }
        };
        self.prompt_pending = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Remove {name}?"),
            Some(detail),
            &["Cancel", "Remove"],
            cx,
        );
        self._tasks.push(cx.spawn(async move |view, cx| {
            let answer = answer.await;
            let _ = view.update(cx, |this, _| {
                this.prompt_pending = false;
                if answer == Ok(1) {
                    this.core.send(match target {
                        RemovalTarget::Project(id) => CoreCommand::RemoveProject(id),
                        RemovalTarget::Session(id) => CoreCommand::RemoveSession(id),
                    });
                }
            });
        }));
    }
    fn agent_rows(&self, project: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut rows = v_flex()
            .gap_1()
            .child(div().pl_3().pt_2().text_xs().child("Agents"));
        let mut agents: Vec<_> = self
            .tabs
            .iter()
            .filter(|(_, tab)| tab.info.agent.is_some())
            .collect();
        agents.sort_by_key(|(id, tab)| {
            (
                self.side_parent(**id).unwrap_or(**id),
                is_side(&tab.info),
                **id,
            )
        });
        for (id, tab) in agents {
            if tab.info.project != project {
                continue;
            }
            let id = *id;
            let Some(agent) = &tab.info.agent else {
                continue;
            };
            let running = matches!(
                tab.info.status,
                SessionStatus::Starting | SessionStatus::Running | SessionStatus::Stopping
            );
            let status = if matches!(tab.info.status, SessionStatus::Stopping) {
                "Stopping"
            } else {
                match agent.status {
                    agent::Status::Launching => "Launching",
                    agent::Status::Idle => "Idle",
                    agent::Status::Working => "Working",
                    agent::Status::NeedsApproval => "Needs approval",
                    agent::Status::TurnFailed => "Turn failed",
                    agent::Status::Attention => "Attention",
                    agent::Status::ActiveInferred => "Active (inferred)",
                    agent::Status::Exited => "Resume",
                }
            };
            rows = rows.child(
                h_flex()
                    .pl_3()
                    .when(is_side(&tab.info), |row| row.pl_6())
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_1()
                            .id(("agent", id.0))
                            .flex_1()
                            .text_sm()
                            .cursor_pointer()
                            .when(agent.status == agent::Status::NeedsApproval, |div| {
                                div.text_color(theme.warning)
                            })
                            .child(
                                div()
                                    .text_color(match agent.status {
                                        agent::Status::NeedsApproval
                                        | agent::Status::TurnFailed
                                        | agent::Status::Attention => theme.warning,
                                        agent::Status::Working
                                        | agent::Status::Launching
                                        | agent::Status::ActiveInferred => theme.primary,
                                        _ => theme.muted_foreground,
                                    })
                                    .child("●"),
                            )
                            .child(format!(
                                "{}{} · {status}",
                                if is_side(&tab.info) {
                                    "↳ Read-only · "
                                } else {
                                    ""
                                },
                                tab.info.name
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_tab(id, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(("agent-toggle", id.0))
                            .text_xs()
                            .cursor_pointer()
                            .child(if running { "Stop" } else { "Resume" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_tab(id, window, cx);
                                this.core.send(if running {
                                    CoreCommand::Stop(id)
                                } else {
                                    CoreCommand::ResumeAgent(id)
                                });
                            })),
                    )
                    .child(
                        div()
                            .id(("remove-agent", id.0))
                            .text_xs()
                            .cursor_pointer()
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_remove(RemovalTarget::Session(id), window, cx);
                            })),
                    ),
            );
        }
        for provider in [agent::Provider::Claude, agent::Provider::Codex] {
            let project = project.to_string();
            rows = rows.child(
                div()
                    .id(SharedString::from(format!(
                        "launch-{}-{project}",
                        provider.label()
                    )))
                    .pl_3()
                    .py_1()
                    .text_sm()
                    .cursor_pointer()
                    .child(format!("+ Launch {}", provider.label()))
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.core.send(CoreCommand::NewAgent {
                            project: project.clone(),
                            picker: false,
                            provider,
                        })
                    })),
            );
        }
        rows.into_any_element()
    }

    fn tab_key(tab: &ShellTab) -> String {
        if let Some(agent) = &tab.info.agent {
            format!("agent:{}", agent.record.id)
        } else if let Some(process) = &tab.info.process {
            format!("process:{}/{}", tab.info.project, process)
        } else {
            format!("shell:{}", tab.info.id.0)
        }
    }
    fn save_layout(&mut self) {
        if self.restoring || self.shutdown.is_some() || !self.perf_layout.is_empty() {
            return;
        }
        let layout = layout::Layout {
            active_project: self.active_project.clone(),
            expanded: self.expanded.iter().cloned().collect(),
            selected: self
                .selected
                .values()
                .filter_map(|id| self.tabs.get(id))
                .map(Self::tab_key)
                .collect(),
            tabs: self
                .tab_order
                .iter()
                .filter_map(|id| self.tabs.get(id))
                .filter(|tab| tab.open || tab.info.process.is_some() || tab.info.agent.is_some())
                .map(|tab| layout::Tab {
                    key: Self::tab_key(tab),
                    project: tab.info.project.clone(),
                    name: tab.info.name.clone(),
                    kind: if let Some(agent) = &tab.info.agent {
                        layout::Kind::Agent(agent.record.id.clone())
                    } else if let Some(process) = &tab.info.process {
                        layout::Kind::Process(process.clone())
                    } else {
                        layout::Kind::Shell
                    },
                    cwd: None,
                    open: tab.open,
                })
                .collect(),
        };
        if self.last_layout.as_ref() != Some(&layout) {
            self.last_layout = Some(layout.clone());
            self.core.send(CoreCommand::SaveLayout(layout));
        }
    }
    fn visible_ids(&self) -> Vec<SessionId> {
        let mut ids = self.active().into_iter().collect::<Vec<_>>();
        if let Some(side) = self.active().and_then(|id| self.side_panes.get(&id)) {
            ids.push(*side);
        }
        if self.stress.is_some() {
            for id in [
                SessionId(devdesk_core::perf::noisy_count() as u64 + 1),
                SessionId(1),
                SessionId(2),
            ] {
                if ids.len() < 3 && self.tabs.contains_key(&id) && !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids
    }
    fn start_stress(&mut self) {
        if self.config.projects.len() < 3 {
            return;
        }
        let Some(stress) = self.stress.as_mut() else {
            return;
        };
        if stress.requested {
            return;
        }
        stress.requested = true;
        self.active_project = Some("probe".into());
        for index in 0..devdesk_core::perf::noisy_count() + 1 {
            self.core.send(CoreCommand::NewShell {
                project: ["probe", "probe2", "probe3"][index % 3].into(),
                size: devdesk_term::Dimensions::default(),
            });
        }
    }
    fn schedule_stress(&self, window: &mut Window, cx: &Context<Self>) {
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let dispatch = weak
                .update(cx, |this, cx| this.stress_frame(window, cx))
                .ok()
                .flatten();
            // Dispatch can synchronously draw the root. Release Workspace's
            // entity lease first, just as a real platform key event does.
            if let Some((focus, marker)) = dispatch {
                focus.focus(window, cx);
                window.dispatch_keystroke(Keystroke::parse(&marker).expect("a-t is a key"), cx);
            }
        });
    }
    fn stress_frame(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(FocusHandle, String)> {
        let stress = self.stress.as_mut()?;
        let age = stress.age();
        let mut dispatch = None;
        let now = std::time::Instant::now();
        if let Some(last) = stress.last_frame.replace(now)
            && age >= std::time::Duration::from_secs(1)
        {
            stress.frames.push((now - last).as_secs_f64() * 1000.);
        }
        if age > std::time::Duration::from_secs(4) {
            eprintln!("devdesk-many-progress requesting shutdown");
            stress.shutdown = Some(now);
            self.shutdown = Some(self.tabs.len());
            self.core.send(CoreCommand::Shutdown);
            cx.notify();
            return None;
        }
        let switch =
            age.as_millis() >= 100 + stress.switch_count as u128 * 100 && stress.switch_count < 10;
        if switch {
            let project = ["probe", "probe2", "probe3"][stress.switch_count % 3].to_owned();
            stress.switch_count += 1;
            let started = std::time::Instant::now();
            self.open_project(project, window, cx);
            let weak = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = weak.update(cx, |this, _| {
                    if let Some(stress) = this.stress.as_mut() {
                        stress
                            .switches
                            .push(started.elapsed().as_secs_f64() * 1000.);
                    }
                });
            });
        }
        if age >= std::time::Duration::from_millis(1200)
            && !self
                .stress
                .as_ref()
                .is_some_and(|stress| stress.palette_started)
        {
            self.stress.as_mut().expect("stress probe").palette_started = true;
            let started = std::time::Instant::now();
            self.show_palette(&Palette, window, cx);
            let weak = cx.weak_entity();
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.stress.as_mut().expect("stress probe").palette_ms =
                        Some(started.elapsed().as_secs_f64() * 1000.);
                    this.palette
                        .update(cx, |state, cx| state.set_query("swpj", window, cx));
                });
                window.on_next_frame(move |window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let matches = this.palette_actions(cx).len();
                        this.stress.as_mut().expect("stress probe").fuzzy_matches = matches;
                        this.palette_open = false;
                        this.restore_focus(window, cx);
                        cx.notify();
                    });
                });
            });
        }
        if age >= std::time::Duration::from_secs(2)
            && let Some(tab) = self
                .tabs
                .get(&SessionId(devdesk_core::perf::noisy_count() as u64 + 1))
        {
            let text: String = tab
                .view
                .read(cx)
                .snapshot
                .as_ref()
                .into_iter()
                .flat_map(|snapshot| snapshot.runs.iter().map(|run| run.text.as_str()))
                .collect();
            let stress = self.stress.as_mut().expect("active stress probe");
            if let Some((started, marker)) = &stress.pending_key
                && text.contains(marker)
            {
                stress.echoes.push(started.elapsed().as_secs_f64() * 1000.);
                stress.pending_key = None;
            }
            if stress.echoes.len() < 20 && stress.pending_key.is_none() && text.contains("READY") {
                let marker = ((b'a' + stress.echoes.len() as u8) as char).to_string();
                stress.pending_key = Some((std::time::Instant::now(), marker.clone()));
                let focus = tab.view.read(cx).focus.clone();
                eprintln!("devdesk-many-progress dispatching {marker}");
                dispatch = Some((focus, marker));
            }
        }
        self.schedule_stress(window, cx);
        dispatch
    }
    fn update_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let visible = self.visible_ids();
        // Hide first so the previous pane cannot write to the clipboard while
        // the next pane takes focus. Input still goes directly to the PTY.
        for (id, tab) in &self.tabs {
            if !visible.contains(id) {
                tab.view
                    .update(cx, |view, cx| view.set_visible(false, window, cx));
            }
        }
        for id in visible {
            if let Some(tab) = self.tabs.get(&id) {
                tab.view
                    .update(cx, |view, cx| view.set_visible(true, window, cx));
            }
        }
        self.restore_focus(window, cx);
    }

    fn open_project(&mut self, project: String, window: &mut Window, cx: &mut Context<Self>) {
        let started = std::env::var_os("DEVDESK_PERF_SWITCH").map(|_| std::time::Instant::now());
        self.expanded.insert(project.clone());
        self.active_project = Some(project.clone());
        if self.ready {
            self.core.send(CoreCommand::ProjectOpened(project.clone()));
        }
        if self.ready
            && !self.tabs.values().any(|tab| {
                tab.info.project == project
                    && tab.info.process.is_none()
                    && tab.info.agent.is_none()
            })
            && !self.pending.contains(&project)
        {
            self.pending.insert(project.clone());
            self.core.send(CoreCommand::NewShell {
                project,
                size: devdesk_term::Dimensions::default(),
            });
        }
        self.update_visibility(window, cx);
        self.save_layout();
        cx.notify();
        if let Some(started) = started {
            window.on_next_frame(move |_, _| {
                eprintln!(
                    "devdesk-switch frame_ms={:.3}",
                    started.elapsed().as_secs_f64() * 1000.
                )
            });
        }
    }
    fn side_parent(&self, id: SessionId) -> Option<SessionId> {
        let parent = self
            .tabs
            .get(&id)?
            .info
            .agent
            .as_ref()?
            .record
            .parent
            .as_ref()?;
        self.tabs
            .iter()
            .find(|(_, tab)| {
                tab.info
                    .agent
                    .as_ref()
                    .is_some_and(|info| &info.record.id == parent)
            })
            .map(|(id, _)| *id)
    }
    fn open_side(&mut self, id: SessionId) {
        let Some(parent) = self.side_parent(id) else {
            return;
        };
        if let Some(previous) = self.side_panes.insert(parent, id)
            && previous != id
        {
            self.core.send(CoreCommand::Close {
                id: previous,
                confirmed: None,
            });
        }
        self.side_focus = Some(id);
        let project = self.tabs[&parent].info.project.clone();
        self.tabs.get_mut(&parent).unwrap().open = true;
        self.active_project = Some(project.clone());
        self.selected.insert(project, parent);
    }
    fn choose_tab(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.get(&id).is_some_and(|tab| is_side(&tab.info)) {
            self.open_side(id);
            self.update_visibility(window, cx);
            cx.notify();
            return;
        }
        self.side_focus = None;
        if let Some(tab) = self.tabs.get_mut(&id) {
            tab.open = true;
            self.active_project = Some(tab.info.project.clone());
            self.selected.insert(tab.info.project.clone(), id);
            self.update_visibility(window, cx);
            self.save_layout();
            cx.notify();
        }
    }
    fn pick_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.project_tabs().get(index).copied() {
            self.choose_tab(id, window, cx);
        }
    }
    fn step_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = self.project_tabs();
        if tabs.is_empty() {
            return;
        }
        let index = tabs
            .iter()
            .position(|id| Some(*id) == self.active())
            .unwrap_or(0);
        self.choose_tab(
            tabs[(index as isize + offset).rem_euclid(tabs.len() as isize) as usize],
            window,
            cx,
        );
    }
    fn new_terminal(&mut self, _: &NewTerminal, _: &mut Window, _: &mut Context<Self>) {
        if self.shutdown.is_none()
            && let Some(project) = &self.active_project
        {
            self.core.send(CoreCommand::NewShell {
                project: project.clone(),
                size: devdesk_term::Dimensions::default(),
            });
        }
    }
    fn close_terminal(&mut self, _: &CloseTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.is_some() || self.question_parent.is_some() || self.palette_open {
            self.question_parent = None;
            self.rename = None;
            self.palette_open = false;
            self.restore_focus(window, cx);
            cx.notify();
            return;
        }
        let focused_side = self
            .active()
            .and_then(|id| self.side_panes.get(&id))
            .copied()
            .filter(|id| self.tabs[id].view.read(cx).focus.is_focused(window));
        if let Some(id) = focused_side.or_else(|| self.active()) {
            self.close_tab(id, window, cx);
        }
    }
    fn close_tab(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(side) = self.side_panes.remove(&id) {
            self.core.send(CoreCommand::Close {
                id: side,
                confirmed: None,
            });
        }
        self.side_panes.retain(|_, side| *side != id);
        self.side_focus = None;
        self.core.send(CoreCommand::Close {
            id,
            confirmed: None,
        });
        let Some(tab) = self.tabs.get_mut(&id) else {
            return;
        };
        if tab.info.process.is_none() && tab.info.agent.is_none() {
            // Shells stay visible until any foreground-job confirmation and
            // session cleanup complete. Processes keep a permanent core entry.
            return;
        }
        tab.open = false;
        let project = tab.info.project.clone();
        if self.selected.get(&project) == Some(&id) {
            let next = self
                .tabs
                .iter()
                .find(|(_, candidate)| {
                    candidate.open && !is_side(&candidate.info) && candidate.info.project == project
                })
                .map(|(id, _)| *id);
            if let Some(next) = next {
                self.selected.insert(project, next);
            } else {
                self.selected.remove(&project);
            }
        }
        self.update_visibility(window, cx);
        self.save_layout();
        cx.notify();
    }
    fn add_project(&mut self, _: &AddProject, _: &mut Window, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add Project".into()),
        });
        self._tasks
            .push(cx.spawn(async move |view, cx| match answer.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(folder) = paths.into_iter().next() {
                        let _ = view.update(cx, |this, _| {
                            this.core.send(CoreCommand::AddProject(folder))
                        });
                    }
                }
                Ok(Err(error)) => {
                    let _ = view.update(cx, |this, cx| {
                        this.error = Some(error.to_string());
                        cx.notify();
                    });
                }
                _ => {}
            }));
    }
    fn ask_agent_question(
        &mut self,
        _: &AskAgentQuestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.active() else { return };
        let tab = &self.tabs[&id];
        if !tab.info.agent.as_ref().is_some_and(|agent| {
            agent.record.role == agent::Role::Primary
                && agent.record.session_id.is_some()
                && agent.status != agent::Status::Launching
        }) || tab.info.status != SessionStatus::Running
        {
            self.error = Some("Select a running agent with a known session ID".into());
            cx.notify();
            return;
        }
        self.rename = None;
        self.question_parent = Some(id);
        self.rename_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }
    fn rename_terminal(&mut self, _: &RenameTerminal, window: &mut Window, cx: &mut Context<Self>) {
        let side = self
            .active()
            .and_then(|id| self.side_panes.get(&id))
            .copied()
            .filter(|id| self.tabs[id].view.read(cx).focus.is_focused(window));
        if let Some(id) = side.or_else(|| self.active())
            && let Some(tab) = self.tabs.get(&id)
        {
            self.question_parent = None;
            self.rename = Some(id);
            self.rename_input.update(cx, |input, cx| {
                input.set_value(tab.info.name.clone(), window, cx);
                input.focus(window, cx);
            });
            cx.notify();
        }
    }
    fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = self.question_parent.take() {
            self.core.send(CoreCommand::ForkAgent {
                parent,
                question: self.rename_input.read(cx).value().to_string(),
            });
        }
        if let Some(id) = self.rename.take() {
            self.core.send(CoreCommand::Rename {
                id,
                name: self.rename_input.read(cx).value().to_string(),
            });
        }
        self.restore_focus(window, cx);
        cx.notify();
    }
    fn restore_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .side_focus
            .filter(|id| self.visible_ids().contains(id))
            .or_else(|| self.active())
            .and_then(|id| self.tabs.get(&id))
            .map(|tab| tab.view.read(cx).focus.clone())
            .unwrap_or_else(|| self.focus.clone());
        focus.focus(window, cx);
    }
    fn show_palette(&mut self, _: &Palette, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette.update(cx, |state, cx| {
            state.set_query("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
    fn palette_actions(&self, cx: &App) -> Vec<(String, PaletteAction)> {
        let mut actions = vec![
            ("New Terminal".into(), PaletteAction::New),
            ("Close Terminal".into(), PaletteAction::Close),
            ("Rename Terminal".into(), PaletteAction::Rename),
            ("Add Project…".into(), PaletteAction::Add),
            ("Ask Agent Question…".into(), PaletteAction::Ask),
        ];
        actions.extend(self.projects.iter().map(|project| {
            (
                format!("Switch Project: {}", project.name),
                PaletteAction::Project(project.id.clone()),
            )
        }));
        if let Some(project) = &self.active_project {
            let provider = self
                .projects
                .iter()
                .find(|item| item.id == *project)
                .and_then(|item| item.agents.default_provider.as_deref());
            actions.push((
                "Launch Default Agent".into(),
                PaletteAction::LaunchAgent(
                    if provider == Some("codex") {
                        agent::Provider::Codex
                    } else {
                        agent::Provider::Claude
                    },
                    false,
                ),
            ));
            actions.push((
                "Launch Claude".into(),
                PaletteAction::LaunchAgent(agent::Provider::Claude, false),
            ));
            actions.push((
                "Resume other Claude session…".into(),
                PaletteAction::LaunchAgent(agent::Provider::Claude, true),
            ));
            actions.push((
                "Launch Codex".into(),
                PaletteAction::LaunchAgent(agent::Provider::Codex, false),
            ));
            actions.push((
                "Resume other Codex session…".into(),
                PaletteAction::LaunchAgent(agent::Provider::Codex, true),
            ));
            for (id, tab) in &self.tabs {
                if tab.info.project == *project && tab.info.agent.is_some() {
                    actions.push((
                        format!("Resume {}", tab.info.name),
                        PaletteAction::ResumeAgent(*id),
                    ));
                    actions.push((format!("Stop {}", tab.info.name), PaletteAction::Stop(*id)));
                }
            }
            actions.push((
                "Start All Processes".into(),
                PaletteAction::StartAll(project.clone()),
            ));
            actions.push((
                "Stop All Processes".into(),
                PaletteAction::StopAll(project.clone()),
            ));
            for (id, tab) in &self.tabs {
                if tab.info.project == *project && tab.info.process.is_some() {
                    actions.push((
                        format!("Start {}", tab.info.name),
                        PaletteAction::Start(*id),
                    ));
                    actions.push((format!("Stop {}", tab.info.name), PaletteAction::Stop(*id)));
                    actions.push((
                        format!("Restart {}", tab.info.name),
                        PaletteAction::Restart(*id),
                    ));
                }
            }
        }
        for project in &self.projects {
            actions.push((
                format!("Remove Project: {}", project.name),
                PaletteAction::Remove(RemovalTarget::Project(project.id.clone())),
            ));
        }
        for (id, tab) in &self.tabs {
            actions.push((
                format!("Open Tab: {}", tab.info.name),
                PaletteAction::Select(*id),
            ));
            if tab.info.agent.is_some() || tab.info.process.is_some() {
                actions.push((
                    format!("Remove: {}", tab.info.name),
                    PaletteAction::Remove(RemovalTarget::Session(*id)),
                ));
            }
        }
        actions.push(("Focus Primary Terminal".into(), PaletteAction::FocusPrimary));
        actions.push(("Focus Side Conversation".into(), PaletteAction::FocusSide));
        if let Some(parent) = self.active()
            && self.side_panes.contains_key(&parent)
        {
            actions.push((
                "Widen Side Conversation".into(),
                PaletteAction::ResizeSide(50),
            ));
            actions.push((
                "Narrow Side Conversation".into(),
                PaletteAction::ResizeSide(-50),
            ));
        }
        let mut links = BTreeSet::new();
        for id in self.visible_ids() {
            if let Some(snapshot) = self.tabs[&id].view.read(cx).snapshot.as_ref() {
                for link in &snapshot.hyperlinks {
                    if link.uri.starts_with("https://")
                        || link.uri.starts_with("http://")
                        || link.uri.starts_with("file://")
                    {
                        links.insert(link.uri.clone());
                    }
                }
            }
        }
        for uri in links {
            let label: String = uri.chars().take(120).collect();
            actions.push((format!("Open Link: {label}"), PaletteAction::OpenUrl(uri)));
        }
        actions.push(("Reload Configuration".into(), PaletteAction::Reload));
        actions.push(("Quit DevDesk".into(), PaletteAction::Quit));
        for project in &self.projects {
            actions.push((
                format!("Toggle Project Details: {}", project.name),
                PaletteAction::ToggleProject(project.id.clone()),
            ));
        }
        if let Some(id) = self.active() {
            actions.push(("Restart Active Terminal".into(), PaletteAction::Restart(id)));
        }
        actions.push(("Recover Previous Sessions…".into(), PaletteAction::Recover));
        actions
            .into_iter()
            .filter(|(label, _)| fuzzy_match(label, &self.palette_query))
            .collect()
    }
    fn confirm_palette(
        &mut self,
        action: PaletteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.palette_open = false;
        match action {
            PaletteAction::Project(project) => self.open_project(project, window, cx),
            PaletteAction::New => self.new_terminal(&NewTerminal, window, cx),
            PaletteAction::Close => self.close_terminal(&CloseTerminal, window, cx),
            PaletteAction::Rename => self.rename_terminal(&RenameTerminal, window, cx),
            PaletteAction::Add => self.add_project(&AddProject, window, cx),
            PaletteAction::ResizeSide(delta) => {
                if let Some(parent) = self.active()
                    && let Some(state) = self.split_states.get(&parent).cloned()
                {
                    state.update(cx, |state, cx| {
                        if let Some(size) = state.sizes().get(1).copied() {
                            state.resize_panel(1, size + px(delta as f32), window, cx);
                        }
                    });
                }
            }
            PaletteAction::OpenUrl(uri) => cx.open_url(&uri),
            PaletteAction::Reload => self.core.send(CoreCommand::ReloadConfig),
            PaletteAction::Quit => self.request_quit(window, cx),
            PaletteAction::ToggleProject(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            PaletteAction::Remove(target) => self.confirm_remove(target, window, cx),
            PaletteAction::Select(id) => self.choose_tab(id, window, cx),
            PaletteAction::FocusPrimary => {
                self.side_focus = None;
            }
            PaletteAction::FocusSide => {
                self.side_focus = self
                    .active()
                    .and_then(|id| self.side_panes.get(&id))
                    .copied();
            }
            PaletteAction::Ask => self.ask_agent_question(&AskAgentQuestion, window, cx),
            PaletteAction::Start(id) => self.core.send(CoreCommand::Start(id)),
            PaletteAction::Stop(id) => self.core.send(CoreCommand::Stop(id)),
            PaletteAction::Restart(id) => self.core.send(CoreCommand::Restart(id)),
            PaletteAction::StartAll(project) => self.core.send(CoreCommand::StartAll(project)),
            PaletteAction::StopAll(project) => self.core.send(CoreCommand::StopAll(project)),
            PaletteAction::Recover => self.core.send(CoreCommand::ShowOrphans),
            PaletteAction::LaunchAgent(provider, picker) => {
                if let Some(project) = &self.active_project {
                    self.core.send(CoreCommand::NewAgent {
                        project: project.clone(),
                        picker,
                        provider,
                    });
                }
            }
            PaletteAction::ResumeAgent(id) => {
                self.choose_tab(id, window, cx);
                self.core.send(CoreCommand::ResumeAgent(id));
            }
        }
        if self.rename.is_none() && self.question_parent.is_none() {
            self.restore_focus(window, cx);
        }
        cx.notify();
    }
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shutdown.is_some() || self.prompt_pending {
            return;
        }
        self.save_layout();
        if let Some(layout) = self.last_layout.clone() {
            self.core.send(CoreCommand::SaveLayout(layout));
        }
        let running: Vec<_> = self
            .tabs
            .values()
            .filter(|tab| {
                matches!(
                    tab.info.status,
                    SessionStatus::Starting | SessionStatus::Running | SessionStatus::Stopping
                )
            })
            .map(|tab| tab.info.name.clone())
            .collect();
        if running.is_empty() {
            self.shutdown = Some(0);
            self.core.send(CoreCommand::Shutdown);
            cx.notify();
            return;
        }
        self.prompt_pending = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Stop {} sessions and quit?", running.len()),
            Some(&running.join("\n")),
            &["Cancel", "Stop all and quit"],
            cx,
        );
        self._tasks.push(cx.spawn(async move |view, cx| {
            let answer = answer.await;
            let _ = view.update(cx, |this, cx| {
                this.prompt_pending = false;
                if answer == Ok(1) {
                    this.shutdown = Some(this.tabs.len());
                    this.core.send(CoreCommand::Shutdown);
                    cx.notify();
                }
            });
        }));
    }
}

fn fuzzy_match(label: &str, query: &str) -> bool {
    let label = label.to_lowercase();
    let mut letters = label.chars();
    query
        .to_lowercase()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .all(|ch| letters.by_ref().any(|candidate| candidate == ch))
}
fn dot(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Running => "●",
        SessionStatus::Starting | SessionStatus::Stopping => "◐",
        _ => "○",
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Split state belongs to this view, rather than render-local keyed
        // state: keyboard actions must resize it after rendering completes.
        if let Some(parent) = self.active()
            && self.side_panes.contains_key(&parent)
        {
            self.split_states
                .entry(parent)
                .or_insert_with(|| cx.new(|_| ResizableState::default()));
        }
        let theme = cx.theme();
        let bg = theme.background;
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let active_bg = theme.accent;
        if self.perf_layout == "minimal" {
            return div().size_full().bg(bg).into_any_element();
        }
        let mut sidebar = v_flex()
            .id("projects-sidebar")
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .p_3()
            .gap_2()
            .border_r_1()
            .border_color(border)
            .child(div().font_bold().child("DevDesk"))
            .child(div().text_xs().text_color(muted).child("PROJECTS"));
        for project in &self.projects {
            let id = project.id.clone();
            let toggle_id = id.clone();
            let remove_id = id.clone();
            let expanded = self.expanded.contains(&id);
            let selected = self.active_project.as_ref() == Some(&id);
            sidebar = sidebar.child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .id(SharedString::from(format!("expand-{id}")))
                            .cursor_pointer()
                            .child(if expanded { "▾" } else { "▸" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&toggle_id) {
                                    this.expanded.insert(toggle_id.clone());
                                }
                                this.save_layout();
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("project-{id}")))
                            .flex_1()
                            .cursor_pointer()
                            .rounded_sm()
                            .p_1()
                            .when(selected, |this| this.bg(active_bg))
                            .child(project.name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_project(id.clone(), window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("remove-project-{remove_id}")))
                            .cursor_pointer()
                            .text_xs()
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_remove(
                                    RemovalTarget::Project(remove_id.clone()),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            );
            if expanded {
                for (id, tab) in &self.tabs {
                    if tab.info.project != project.id {
                        continue;
                    }
                    let id = *id;
                    if tab.info.agent.is_some() {
                        continue;
                    }
                    if tab.info.process.is_some() {
                        let running = matches!(
                            tab.info.status,
                            SessionStatus::Starting
                                | SessionStatus::Running
                                | SessionStatus::Stopping
                        );
                        let failed = matches!(tab.info.status, SessionStatus::Failed(_));
                        let status = match &tab.info.status {
                            SessionStatus::Failed(reason) => reason.clone(),
                            SessionStatus::Stopped => "Stopped".into(),
                            SessionStatus::Starting => "Starting".into(),
                            SessionStatus::Running => "Running".into(),
                            SessionStatus::Stopping => "Stopping".into(),
                            SessionStatus::Exited { .. } => "Exited".into(),
                        };
                        sidebar = sidebar.child(
                            v_flex()
                                .pl_3()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .id(("process", id.0))
                                                .flex_1()
                                                .cursor_pointer()
                                                .text_sm()
                                                .when(failed, |div| div.text_color(theme.danger))
                                                .child(format!(
                                                    "{} {}",
                                                    dot(&tab.info.status),
                                                    tab.info.name
                                                ))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.choose_tab(id, window, cx)
                                                    },
                                                )),
                                        )
                                        .child(
                                            div()
                                                .id(("process-toggle", id.0))
                                                .cursor_pointer()
                                                .text_xs()
                                                .child(if running { "Stop" } else { "Start" })
                                                .on_click(cx.listener(move |this, _, _, _| {
                                                    this.core.send(if running {
                                                        CoreCommand::Stop(id)
                                                    } else {
                                                        CoreCommand::Start(id)
                                                    })
                                                })),
                                        )
                                        .child(
                                            div()
                                                .id(("process-restart", id.0))
                                                .cursor_pointer()
                                                .text_xs()
                                                .child("↻")
                                                .on_click(cx.listener(move |this, _, _, _| {
                                                    this.core.send(CoreCommand::Restart(id))
                                                })),
                                        )
                                        .child(
                                            div()
                                                .id(("remove-process", id.0))
                                                .cursor_pointer()
                                                .text_xs()
                                                .child("×")
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.confirm_remove(
                                                            RemovalTarget::Session(id),
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                        ),
                                )
                                .child(div().text_xs().text_color(muted).child(
                                    if tab.info.config_changed {
                                        format!("{status} · config changed — restart to apply")
                                    } else {
                                        status
                                    },
                                )),
                        );
                        continue;
                    }
                    sidebar = sidebar.child(
                        div()
                            .id(("shell", id.0))
                            .pl_5()
                            .py_1()
                            .text_sm()
                            .cursor_pointer()
                            .child(format!("{} {}", dot(&tab.info.status), tab.info.name))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_tab(id, window, cx)
                            })),
                    );
                }
                sidebar = sidebar.child(self.agent_rows(&project.id, cx));
            }
        }
        sidebar = sidebar.child(
            div()
                .id("add-project")
                .cursor_pointer()
                .py_2()
                .text_sm()
                .child("+ Add Project…")
                .on_click(
                    cx.listener(|this, _, window, cx| this.add_project(&AddProject, window, cx)),
                ),
        );
        let ids = self.project_tabs();
        let selected_index = ids
            .iter()
            .position(|id| Some(*id) == self.active())
            .unwrap_or(0);
        let mut tabs = TabBar::new("shell-tabs").selected_index(selected_index);
        for id in &ids {
            let tab = &self.tabs[id];
            let id = *id;
            tabs = tabs.child(
                Tab::new()
                    .label(format!("{} {}", dot(&tab.info.status), tab.info.name))
                    .suffix(
                        div()
                            .id(("close", id.0))
                            .px_1()
                            .cursor_pointer()
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(id, window, cx);
                            })),
                    ),
            );
        }
        let click_ids = ids.clone();
        tabs = tabs
            .on_click(cx.listener(move |this, index: &usize, window, cx| {
                if let Some(id) = click_ids.get(*index) {
                    this.choose_tab(*id, window, cx);
                }
            }))
            .suffix(
                div()
                    .id("new-terminal")
                    .px_3()
                    .cursor_pointer()
                    .child("+")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_terminal(&NewTerminal, window, cx)
                    })),
            );
        let visible = self.visible_ids();
        let mut content = v_flex().flex_1().min_w_0().h_full().child(tabs);
        if let Some(side) = self
            .active()
            .and_then(|id| self.side_panes.get(&id))
            .copied()
        {
            let parent = self.active().unwrap();
            let record = &self.tabs[&side].info.agent.as_ref().unwrap().record;
            let chrome = format!(
                "Read-only · snapshot at {} · of {}",
                agent::snapshot_label(record.snapshot_at),
                self.tabs[&parent].info.name
            );
            let split_state = self.split_states[&parent].clone();
            let side_pane = v_flex()
                .size_full()
                .min_w_0()
                .min_h_0()
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .text_xs()
                        .text_color(muted)
                        .child(div().flex_1().child(chrome))
                        .child(div().id("close-side").cursor_pointer().child("×").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.close_tab(side, window, cx)
                            }),
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(self.tabs[&side].view.clone()),
                );
            content = content.child(
                div().flex_1().min_h_0().child(
                    h_resizable(("agent-split", parent.0))
                        .with_state(&split_state)
                        .child(resizable_panel().child(self.tabs[&parent].view.clone()))
                        .child(resizable_panel().child(side_pane)),
                ),
            );
        } else if !visible.is_empty() {
            let mut panes = h_flex().flex_1().min_h_0().min_w_0();
            for id in visible {
                if let Some(tab) = self.tabs.get(&id) {
                    panes = panes.child(div().flex_1().min_w_0().h_full().child(tab.view.clone()));
                }
            }
            content = content.child(panes);
        } else {
            content = content.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(if self.active_project.is_some() {
                        "No terminal open · Cmd-T to open one"
                    } else {
                        "No project open"
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("Cmd-P for commands · Add Project to choose a folder"),
                    ),
            );
        }
        let mut body = v_flex()
            .size_full()
            .relative()
            .bg(bg)
            .text_color(fg)
            .track_focus(&self.focus)
            .key_context("Workspace")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && (this.rename.is_some() || this.question_parent.is_some())
                {
                    this.question_parent = None;
                    this.rename = None;
                    this.restore_focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                } else if event.keystroke.key == "escape"
                    && !this.palette_open
                    && let Some(side) = this
                        .active()
                        .and_then(|id| this.side_panes.get(&id))
                        .copied()
                    && this.tabs[&side].view.read(cx).focus.is_focused(window)
                {
                    this.close_tab(side, window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(Self::show_palette))
            .on_action(cx.listener(Self::new_terminal))
            .on_action(cx.listener(Self::close_terminal))
            .on_action(cx.listener(Self::rename_terminal))
            .on_action(cx.listener(Self::ask_agent_question))
            .on_action(cx.listener(Self::add_project))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.step_tab(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousTab, window, cx| this.step_tab(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &Tab1, window, cx| this.pick_tab(0, window, cx)))
            .on_action(cx.listener(|this, _: &Tab2, window, cx| this.pick_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &Tab3, window, cx| this.pick_tab(2, window, cx)))
            .on_action(cx.listener(|this, _: &Tab4, window, cx| this.pick_tab(3, window, cx)))
            .on_action(cx.listener(|this, _: &Tab5, window, cx| this.pick_tab(4, window, cx)))
            .on_action(cx.listener(|this, _: &Tab6, window, cx| this.pick_tab(5, window, cx)))
            .on_action(cx.listener(|this, _: &Tab7, window, cx| this.pick_tab(6, window, cx)))
            .on_action(cx.listener(|this, _: &Tab8, window, cx| this.pick_tab(7, window, cx)))
            .on_action(cx.listener(|this, _: &Tab9, window, cx| this.pick_tab(8, window, cx)));
        if let Some(error) = &self.error {
            body = body.child(div().p_2().text_sm().bg(theme.danger).child(error.clone()));
        }
        body = body.child(
            h_flex()
                .flex_1()
                .min_h_0()
                .when(self.perf_layout != "no-sidebar", |this| this.child(sidebar))
                .child(content),
        );
        if self.palette_open {
            let entries = self.palette_actions(cx);
            let actions: Vec<_> = entries.iter().map(|(_, action)| action.clone()).collect();
            let query_view = cx.weak_entity();
            let confirm_view = query_view.clone();
            let cancel_view = query_view.clone();
            let palette = Command::new(&self.palette)
                .filterable(false)
                .placeholder("Search commands…")
                .items(
                    entries
                        .into_iter()
                        .map(|(label, _)| CommandItem::new().label(label)),
                )
                .on_query(move |query, _, cx| {
                    let _ = query_view.update(cx, |this, cx| {
                        this.palette_query = query.into();
                        cx.notify();
                    });
                })
                .on_confirm(move |index, window, cx| {
                    if let Some(action) = actions.get(index.row) {
                        let _ = confirm_view.update(cx, |this, cx| {
                            this.confirm_palette(action.clone(), window, cx)
                        });
                    }
                })
                .on_cancel(move |window, cx| {
                    let _ = cancel_view.update(cx, |this, cx| {
                        this.palette_open = false;
                        this.restore_focus(window, cx);
                        cx.notify();
                    });
                });
            body = body.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(hsla(0., 0., 0., 0.3))
                    .flex()
                    .justify_center()
                    .pt(px(80.))
                    .child(div().w(px(560.)).h_auto().child(palette)),
            );
        }
        if self.rename.is_some() || self.question_parent.is_some() {
            body = body.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(hsla(0., 0., 0., 0.3))
                    .flex()
                    .justify_center()
                    .pt(px(100.))
                    .child(
                        v_flex()
                            .w(px(400.))
                            .h(px(140.))
                            .p_4()
                            .gap_3()
                            .rounded_lg()
                            .bg(bg)
                            .child(if self.question_parent.is_some() {
                                "Ask Agent Question · read-only snapshot"
                            } else {
                                "Rename terminal"
                            })
                            .child(Input::new(&self.rename_input))
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(
                                        div()
                                            .id("rename-save")
                                            .cursor_pointer()
                                            .child(if self.question_parent.is_some() {
                                                "Ask ↵"
                                            } else {
                                                "Save ↵"
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.save_rename(window, cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("rename-cancel")
                                            .cursor_pointer()
                                            .child("Cancel")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.rename = None;
                                                this.question_parent = None;
                                                this.restore_focus(window, cx);
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    ),
            );
        }
        if let Some(remaining) = self.shutdown {
            body = body.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(hsla(0., 0., 0., 0.5))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .p_6()
                            .rounded_lg()
                            .bg(bg)
                            .child(format!("Stopping {remaining} sessions…")),
                    ),
            );
        }
        body.into_any_element()
    }
}

fn is_side(info: &SessionInfo) -> bool {
    info.agent
        .as_ref()
        .is_some_and(|agent| agent.record.role == agent::Role::Side)
}

#[cfg(test)]
mod tests {
    use super::fuzzy_match;
    #[test]
    fn palette_accepts_noncontiguous_case_insensitive_search() {
        assert!(fuzzy_match("Switch Project: DevDesk", "s p dd"));
        assert!(fuzzy_match("New Terminal", "NWT"));
        assert!(!fuzzy_match("New Terminal", "ntz"));
    }
}
