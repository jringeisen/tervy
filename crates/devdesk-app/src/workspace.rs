//! UI mirror of the supervisor. All terminal entities remain alive across tab
//! and project switches; only selected entities are attached to the render tree.
mod customization_smoke;
mod process_form;
mod side_smoke;
use crate::terminal_view::TerminalView;
use devdesk_core::config::{Config, Project};
use devdesk_core::supervisor::{
    Command as CoreCommand, CoreEvent, SessionId, SessionInfo, SessionStatus, SupervisorHandle,
};
use devdesk_core::{agent, layout};
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, Theme, h_flex, v_flex};
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
    Customize(agent::Provider),
    ReviewCustomization,
    UndoCustomization,
    New,
    TerminalPreset(String, String),
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
    ResizeSidebar(i32),
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
    memory: BTreeMap<SessionId, (devdesk_core::supervisor::LaunchId, Option<u64>)>,
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
    customization_provider: Option<agent::Provider>,
    customization_preview: Option<devdesk_core::customization::Preview>,
    customization_notice: Option<String>,
    /// Only the UI owns split presentation; each PTY remains actor-owned.
    /// UI-owned divider state survives renders; actor storage receives only the
    /// final width on mouse-up, never every pointer movement.
    sidebar_state: Entity<ResizableState>,
    sidebar_width: f32,
    split_states: BTreeMap<SessionId, Entity<ResizableState>>,
    side_panes: BTreeMap<SessionId, SessionId>,
    side_focus: Option<SessionId>,
    rename_input: Entity<InputState>,
    process_project: Option<String>,
    process_name_input: Entity<InputState>,
    process_command_input: Entity<InputState>,
    process_error: Option<String>,
    process_saving: bool,
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
        let process_name_input = cx.new(|cx| InputState::new(window, cx));
        let process_command_input = cx.new(|cx| InputState::new(window, cx));
        let name_subscription =
            cx.subscribe_in(&process_name_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.process_command_input
                        .update(cx, |input, cx| input.focus(window, cx));
                }
            });
        let command_subscription =
            cx.subscribe_in(&process_command_input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_process(cx);
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
                if this.perf_layout == "quit" {
                    let weak = cx.weak_entity();
                    window.on_next_frame(move |window, cx| {
                        window.dispatch_keystroke(Keystroke::parse("cmd-q").unwrap(), cx);
                        window.on_next_frame(move |_, cx| {
                            let _ = weak.update(cx, |this, _| {
                                assert!(this.shutdown.is_some(), "Cmd-Q did not request shutdown");
                            });
                        });
                    });
                    window.refresh();
                    return;
                }
                if this.perf_layout == "customization" {
                    if !this.config.projects.is_empty() {
                        this.smoke_customization(window, cx);
                    }
                    return;
                }
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
            memory: BTreeMap::new(),
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
            customization_provider: None,
            customization_preview: None,
            customization_notice: None,
            sidebar_state: cx.new(|_| ResizableState::default()),
            sidebar_width: 280.,
            split_states: BTreeMap::new(),
            side_panes: BTreeMap::new(),
            side_focus: None,
            rename_input,
            process_project: None,
            process_name_input,
            process_command_input,
            process_error: None,
            process_saving: false,
            prompt_pending: false,
            shutdown: None,
            _tasks: vec![task],
            _subscriptions: vec![subscription, name_subscription, command_subscription],
        }
    }
    fn empty_control(&self) -> bool {
        matches!(
            self.perf_layout.as_str(),
            "normal"
                | "minimal"
                | "no-sidebar"
                | "terminal-ui"
                | "idle20"
                | "scrollback20"
                | "quit"
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
            CoreEvent::MemoryUsage(samples) => {
                for (id, launch, bytes) in samples {
                    if self
                        .tabs
                        .get(&id)
                        .is_some_and(|tab| tab.info.launch == launch)
                    {
                        self.memory.insert(id, (launch, bytes));
                    }
                }
            }
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
                    self.sidebar_width = self
                        .config
                        .settings
                        .sidebar_width
                        .or(layout.sidebar_width)
                        .filter(|w| w.is_finite())
                        .unwrap_or(280.)
                        .clamp(220., 520.);
                    self.sidebar_state = cx.new(|_| ResizableState::default());
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
            CoreEvent::CustomizationApplied { undone } => {
                self.customization_notice = Some(if undone {
                    "Previous configuration restored. Restart changed running processes to use it."
                } else {
                    "Customization applied. Undo is available in Cmd-P. Restart changed running processes to use their new definitions."
                }.into());
            }
            CoreEvent::CustomizationPreview(proposal) => {
                self.customization_preview = Some(proposal);
                self.error = None;
            }
            CoreEvent::Config(config) => {
                crate::theme::configure(&config.settings.theme, window, cx);
                for tab in self.tabs.values() {
                    tab.view
                        .update(cx, |view, cx| view.configure(&config.settings, cx));
                }
                if config.settings.sidebar_width != self.config.settings.sidebar_width {
                    self.sidebar_width = config
                        .settings
                        .sidebar_width
                        .or_else(|| self.last_layout.as_ref().and_then(|l| l.sidebar_width))
                        .unwrap_or(280.)
                        .clamp(220., 520.);
                    let width = self.sidebar_width;
                    self.sidebar_state.update(cx, |state, cx| {
                        if !state.sizes().is_empty() {
                            state.resize_panel(0, px(width), window, cx);
                        }
                    });
                }
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
                if self.ready && self.perf_layout == "customization" {
                    self.smoke_customization(window, cx);
                }
            }
            CoreEvent::ConfigError(error) => self.error = Some(error),
            CoreEvent::ProcessAdded(project) => {
                if self.process_project.as_ref() == Some(&project) {
                    self.process_project = None;
                    self.process_error = None;
                    self.process_saving = false;
                    self.restore_focus(window, cx);
                }
            }
            CoreEvent::ProcessAddFailed { project, error } => {
                if self.process_project.as_ref() == Some(&project) {
                    self.process_saving = false;
                    self.process_error = Some(error);
                } else {
                    self.error = Some(error);
                }
            }
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
                self.memory.remove(&id);
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
                if self.perf_layout == "quit" {
                    assert!(self.shutdown.is_some() && clean);
                    eprintln!(
                        "devdesk-quit-smoke PASS shortcut_requested_shutdown=true clean_shutdown=true"
                    );
                }
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
    fn memory_label(&self, info: &SessionInfo, cx: &Context<Self>) -> AnyElement {
        let bytes = self
            .memory
            .get(&info.id)
            .filter(|(launch, _)| {
                *launch == info.launch
                    && matches!(
                        info.status,
                        SessionStatus::Running | SessionStatus::Stopping
                    )
            })
            .and_then(|(_, bytes)| *bytes);
        let label = bytes.map_or_else(
            || "—".to_string(),
            |bytes| format!("{:.0} MB", bytes as f64 / 1_000_000.),
        );
        div().id(("session-memory", info.id.0)).flex_shrink_0()
            .text_xs().text_color(cx.theme().muted_foreground).child(label)
            .tooltip(|window, cx| Tooltip::new("Process memory including children · sampled every 5s · excludes DevDesk scrollback").build(window, cx))
            .into_any_element()
    }

    fn terminal_rows(&self, project: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let add_project = project.to_string();
        let mut rows = v_flex().gap_1().min_w_0().child(
            h_flex()
                .pl_6()
                .pr_2()
                .pt_3()
                .pb_1()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.muted_foreground)
                        .child("TERMINALS"),
                )
                .child(
                    Button::new(SharedString::from(format!("add-terminal-{project}")))
                        .icon(IconName::Plus)
                        .xsmall()
                        .ghost()
                        .tooltip("New terminal")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.active_project = Some(add_project.clone());
                            this.new_terminal(&NewTerminal, window, cx);
                            this.update_visibility(window, cx);
                            this.save_layout();
                            cx.notify();
                        })),
                ),
        );
        // Keep shell order consistent with keyboard navigation and restoration.
        for id in &self.tab_order {
            let Some(tab) = self.tabs.get(id) else {
                continue;
            };
            if tab.info.project != project
                || tab.info.process.is_some()
                || tab.info.agent.is_some()
                || !tab.open
            {
                continue;
            }
            let id = *id;
            let name = tab.info.name.clone();
            rows = rows.child(
                h_flex()
                    .id(("terminal-row", id.0))
                    .ml_4()
                    .px_2()
                    .py_2()
                    .gap_1()
                    .min_w_0()
                    .rounded_md()
                    .when(self.active() == Some(id), |row| {
                        row.bg(theme.sidebar_accent)
                    })
                    .hover(|style| style.bg(theme.sidebar_accent))
                    .child(
                        h_flex()
                            .id(("shell", id.0))
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .text_sm()
                            .cursor_pointer()
                            .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
                            .child(status_indicator(&tab.info, theme))
                            .child(div().min_w_0().truncate().child(tab.info.name.clone()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_tab(id, window, cx)
                            })),
                    )
                    .child(self.memory_label(&tab.info, cx))
                    .child(
                        div()
                            .id(("close-terminal", id.0))
                            .px_1()
                            .py_1()
                            .rounded_sm()
                            .flex_shrink_0()
                            .cursor_pointer()
                            .text_color(theme.muted_foreground)
                            .hover(|style| style.bg(theme.border).text_color(theme.foreground))
                            .tooltip(|window, cx| Tooltip::new("Close terminal").build(window, cx))
                            .child(Icon::new(IconName::Close).size_3())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_tab(id, window, cx)
                            })),
                    ),
            );
        }
        rows.into_any_element()
    }

    fn agent_rows(&self, project: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let launch_project = project.to_string();
        let workspace = cx.weak_entity();
        let mut rows = v_flex().gap_1().min_w_0().child(
            h_flex()
                .pl_6()
                .pr_2()
                .pt_3()
                .pb_1()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.muted_foreground)
                        .child("AGENTS"),
                )
                .child(
                    Button::new(SharedString::from(format!("add-agent-{project}")))
                        .icon(IconName::Plus)
                        .xsmall()
                        .ghost()
                        .tooltip("Launch agent")
                        .dropdown_menu(move |mut menu, _, _| {
                            for provider in [agent::Provider::Claude, agent::Provider::Codex] {
                                let workspace = workspace.clone();
                                let project = launch_project.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(format!("Launch {}", provider.label()))
                                        .on_click(move |_, window, cx| {
                                            let _ = workspace.update(cx, |this, cx| {
                                                if this.shutdown.is_some() {
                                                    return;
                                                }
                                                this.active_project = Some(project.clone());
                                                this.core.send(CoreCommand::NewAgent {
                                                    project: project.clone(),
                                                    picker: false,
                                                    provider,
                                                });
                                                this.update_visibility(window, cx);
                                                this.save_layout();
                                                cx.notify();
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                ),
        );
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
                    agent::Status::Exited => "Exited",
                }
            };
            let full_name = format!(
                "{} · {} · {status}",
                tab.info.name,
                if is_side(&tab.info) {
                    "Read-only"
                } else {
                    agent.record.provider.label()
                }
            );
            rows = rows.child(
                // GPUI retains hover state only for identified elements. Keep
                // it on the entire row so padding and controls repaint too.
                h_flex()
                    .id(("agent-row", id.0))
                    .ml_4()
                    .px_2()
                    .py_2()
                    .min_w_0()
                    .rounded_md()
                    .when(is_side(&tab.info), |row| row.ml_6())
                    .when(
                        self.active() == Some(id) || self.side_focus == Some(id),
                        |row| row.bg(theme.sidebar_accent),
                    )
                    .hover(|style| style.bg(theme.sidebar_accent))
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .id(("agent", id.0))
                            .tooltip(move |window, cx| {
                                Tooltip::new(full_name.clone()).build(window, cx)
                            })
                            .flex_1()
                            .text_sm()
                            .cursor_pointer()
                            .when(agent.status == agent::Status::NeedsApproval, |div| {
                                div.text_color(theme.warning)
                            })
                            .child(status_indicator(&tab.info, theme))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .child(tab.info.name.clone()),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_tab(id, window, cx)
                            })),
                    )
                    .child(self.memory_label(&tab.info, cx))
                    .child(
                        div()
                            .id(("agent-toggle", id.0))
                            .text_xs()
                            .flex_shrink_0()
                            .px_1()
                            .py_1()
                            .rounded_sm()
                            .text_color(theme.muted_foreground)
                            .hover(|style| style.bg(theme.border).text_color(theme.foreground))
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
                            .tooltip(|window, cx| Tooltip::new("Remove agent").build(window, cx))
                            .text_xs()
                            .flex_shrink_0()
                            .px_1()
                            .py_1()
                            .rounded_sm()
                            .text_color(theme.muted_foreground)
                            .hover(|style| style.bg(theme.border).text_color(theme.foreground))
                            .cursor_pointer()
                            .child(Icon::new(IconName::Close).size_3())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_remove(RemovalTarget::Session(id), window, cx);
                            })),
                    ),
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
            // Keep the saved drag preference beneath an explicit config override
            // so removing/undoing that override restores the previous width.
            sidebar_width: if self.config.settings.sidebar_width.is_some() {
                self.last_layout
                    .as_ref()
                    .and_then(|layout| layout.sidebar_width)
            } else {
                Some(self.sidebar_width)
            },
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
        if self.process_project.is_some() {
            self.cancel_process(window, cx);
            return;
        }
        if self.rename.is_some()
            || self.question_parent.is_some()
            || self.customization_provider.is_some()
            || self.customization_preview.is_some()
            || self.palette_open
        {
            self.customization_provider = None;
            self.customization_preview = None;
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
    fn customize(
        &mut self,
        provider: agent::Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_project.is_none() {
            self.error =
                Some("Add or select a project to open a customization conversation".into());
            return;
        }
        self.rename = None;
        self.question_parent = None;
        self.customization_provider = Some(provider);
        self.rename_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }
    fn ask_agent_question(
        &mut self,
        _: &AskAgentQuestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.active() else { return };
        let tab = &self.tabs[&id];
        let problem = match &tab.info.agent {
            None => Some("Select a Claude or Codex agent tab, rather than a shell or process tab"),
            Some(agent) if agent.record.role != agent::Role::Primary => {
                Some("Select the original agent to start another side conversation")
            }
            Some(_) if tab.info.status != SessionStatus::Running => {
                Some("Resume or start the selected agent before asking a side question")
            }
            Some(agent) if agent.status == agent::Status::Launching => {
                Some("The agent is still launching; wait until it is ready")
            }
            Some(agent) if agent.record.session_id.is_none() => {
                Some("The agent has not reported its session ID yet; complete a turn and try again")
            }
            Some(_) => None,
        };
        if let Some(problem) = problem {
            self.error = Some(problem.into());
            cx.notify();
            return;
        }
        self.error = None;
        self.rename = None;
        self.customization_provider = None;
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
            self.customization_provider = None;
            self.rename = Some(id);
            self.rename_input.update(cx, |input, cx| {
                input.set_value(tab.info.name.clone(), window, cx);
                input.focus(window, cx);
            });
            cx.notify();
        }
    }
    fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(provider) = self.customization_provider {
            let request = self.rename_input.read(cx).value().to_string();
            if request.trim().is_empty() {
                self.error = Some("Describe how you want to customize DevDesk".into());
                cx.notify();
                return;
            }
            if let Some(project) = &self.active_project {
                self.core.send(CoreCommand::Customize {
                    project: project.clone(),
                    provider,
                    request,
                });
            }
            self.customization_provider = None;
        }
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
            (
                "Customize DevDesk with Claude…".into(),
                PaletteAction::Customize(agent::Provider::Claude),
            ),
            (
                "Customize DevDesk with Codex…".into(),
                PaletteAction::Customize(agent::Provider::Codex),
            ),
            (
                "Review Customization…".into(),
                PaletteAction::ReviewCustomization,
            ),
            (
                "Undo Last Customization".into(),
                PaletteAction::UndoCustomization,
            ),
            ("Widen Sidebar".into(), PaletteAction::ResizeSidebar(40)),
            ("Narrow Sidebar".into(), PaletteAction::ResizeSidebar(-40)),
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
            if let Some(config) = self.projects.iter().find(|p| p.id == *project) {
                for terminal in &config.terminals {
                    actions.push((
                        format!("Open Terminal Preset: {}", terminal.name),
                        PaletteAction::TerminalPreset(project.clone(), terminal.name.clone()),
                    ));
                }
            }
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
            PaletteAction::Customize(provider) => self.customize(provider, window, cx),
            PaletteAction::ReviewCustomization => self.core.send(CoreCommand::ReviewCustomization),
            PaletteAction::UndoCustomization => self.core.send(CoreCommand::UndoCustomization),
            PaletteAction::Project(project) => self.open_project(project, window, cx),
            PaletteAction::New => self.new_terminal(&NewTerminal, window, cx),
            PaletteAction::TerminalPreset(project, name) => self
                .core
                .send(CoreCommand::NewTerminalPreset { project, name }),
            PaletteAction::Close => self.close_terminal(&CloseTerminal, window, cx),
            PaletteAction::Rename => self.rename_terminal(&RenameTerminal, window, cx),
            PaletteAction::Add => self.add_project(&AddProject, window, cx),
            PaletteAction::ResizeSidebar(delta) => {
                let state = self.sidebar_state.clone();
                state.update(cx, |state, cx| {
                    if let Some(size) = state.sizes().first().copied() {
                        state.resize_panel(0, size + px(delta as f32), window, cx);
                    }
                });
                if let Some(size) = state.read(cx).sizes().first() {
                    self.sidebar_width = f32::from(*size);
                    self.save_layout();
                }
            }
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
        if self.rename.is_none()
            && self.question_parent.is_none()
            && self.customization_provider.is_none()
        {
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

/// Lifecycle takes precedence over agent activity, so an exited or stopping
/// agent cannot look ready because its last hook reported Idle or Working.
fn status_indicator(info: &SessionInfo, theme: &Theme) -> impl IntoElement {
    let color = match &info.status {
        SessionStatus::Failed(_) => theme.danger,
        SessionStatus::Exited {
            code: Some(code), ..
        } if *code != 0 => theme.danger,
        SessionStatus::Stopped | SessionStatus::Exited { .. } => theme.muted_foreground,
        SessionStatus::Starting | SessionStatus::Stopping => theme.warning,
        SessionStatus::Running => match info.agent.as_ref().map(|agent| &agent.status) {
            Some(agent::Status::NeedsApproval | agent::Status::Attention) => theme.warning,
            Some(agent::Status::TurnFailed) => theme.danger,
            Some(agent::Status::Working | agent::Status::ActiveInferred) => theme.info,
            Some(agent::Status::Launching) => theme.warning,
            Some(agent::Status::Exited) => theme.muted_foreground,
            Some(agent::Status::Idle) | None => theme.success,
        },
    };
    div()
        .flex_shrink_0()
        .text_xs()
        .text_color(color)
        .child(dot(&info.status))
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
        let active_bg = theme.sidebar_accent;
        if self.perf_layout == "minimal" {
            return div().size_full().bg(bg).into_any_element();
        }
        let mut sidebar = v_flex()
            .id("projects-sidebar")
            .size_full()
            .min_w_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(
                v_flex()
                    .px_4()
                    .py_4()
                    .gap_1()
                    .child(div().text_sm().font_bold().child("DevDesk"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Developer workspace"),
                    ),
            );
        let mut project_list = v_flex()
            .id("sidebar-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_3()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_2()
                    .text_xs()
                    .font_semibold()
                    .text_color(muted)
                    .child("PROJECTS"),
            );
        for project in &self.projects {
            let id = project.id.clone();
            let toggle_id = id.clone();
            let remove_id = id.clone();
            let full_name = project.name.clone();
            let expanded = self.expanded.contains(&id);
            let selected = self.active_project.as_ref() == Some(&id);
            project_list = project_list.child(
                h_flex()
                    .id(SharedString::from(format!("project-row-{id}")))
                    .hover(|style| style.bg(active_bg))
                    .min_w_0()
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(selected, |row| row.bg(active_bg))
                    .gap_1()
                    .child(
                        div()
                            .id(SharedString::from(format!("expand-{id}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .cursor_pointer()
                            .w(px(18.))
                            .h(px(24.))
                            .text_color(muted)
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size_3(),
                            )
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
                            .tooltip(move |window, cx| {
                                Tooltip::new(full_name.clone()).build(window, cx)
                            })
                            .flex_1()
                            .cursor_pointer()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_semibold()
                            .py_1()
                            .child(project.name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_project(id.clone(), window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("remove-project-{remove_id}")))
                            .tooltip(|window, cx| Tooltip::new("Remove project").build(window, cx))
                            .cursor_pointer()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .text_color(muted)
                            .hover(|style| style.bg(active_bg).text_color(theme.danger))
                            .text_xs()
                            .child(Icon::new(IconName::Close).size_3())
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
                let add_process_project = project.id.clone();
                project_list = project_list.child(
                    h_flex()
                        .pl_6()
                        .pr_2()
                        .pt_3()
                        .pb_1()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .font_semibold()
                                .text_color(muted)
                                .child("PROCESSES"),
                        )
                        .child(
                            Button::new(SharedString::from(format!("add-process-{}", project.id)))
                                .icon(IconName::Plus)
                                .xsmall()
                                .ghost()
                                .tooltip("Add process")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.show_process_form(add_process_project.clone(), window, cx);
                                })),
                        ),
                );
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
                        let full_name = tab.info.name.clone();
                        let detail = if tab.info.config_changed {
                            format!("{status} · config changed — restart to apply")
                        } else {
                            status
                        };
                        project_list = project_list.child(
                            v_flex()
                                .id(("process-row", id.0))
                                .flex_shrink_0()
                                .ml_4()
                                .px_2()
                                .py_2()
                                .rounded_md()
                                .min_w_0()
                                .when(self.active() == Some(id), |row| row.bg(active_bg))
                                .hover(|style| style.bg(active_bg))
                                .gap_1()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            h_flex()
                                                .id(("process", id.0))
                                                .tooltip(move |window, cx| {
                                                    Tooltip::new(format!("{full_name} · {detail}"))
                                                        .build(window, cx)
                                                })
                                                .flex_1()
                                                .min_w_0()
                                                .gap_2()
                                                .cursor_pointer()
                                                .text_sm()
                                                .when(failed, |div| div.text_color(theme.danger))
                                                .child(status_indicator(&tab.info, theme))
                                                .child(
                                                    div()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child(tab.info.name.clone()),
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.choose_tab(id, window, cx)
                                                    },
                                                )),
                                        )
                                        .child(self.memory_label(&tab.info, cx))
                                        .child(
                                            div()
                                                .id(("process-toggle", id.0))
                                                .cursor_pointer()
                                                .flex_shrink_0()
                                                .px_1()
                                                .py_1()
                                                .rounded_sm()
                                                .text_color(muted)
                                                .hover(|style| {
                                                    style.bg(theme.border).text_color(fg)
                                                })
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
                                                .tooltip(|window, cx| {
                                                    Tooltip::new("Restart process")
                                                        .build(window, cx)
                                                })
                                                .cursor_pointer()
                                                .flex_shrink_0()
                                                .px_1()
                                                .py_1()
                                                .rounded_sm()
                                                .text_color(muted)
                                                .hover(|style| {
                                                    style.bg(theme.border).text_color(fg)
                                                })
                                                .text_xs()
                                                .child(Icon::new(IconName::RotateCw).size_3())
                                                .on_click(cx.listener(move |this, _, _, _| {
                                                    this.core.send(CoreCommand::Restart(id))
                                                })),
                                        )
                                        .child(
                                            div()
                                                .id(("remove-process", id.0))
                                                .tooltip(|window, cx| {
                                                    Tooltip::new("Remove process").build(window, cx)
                                                })
                                                .cursor_pointer()
                                                .flex_shrink_0()
                                                .px_1()
                                                .py_1()
                                                .rounded_sm()
                                                .text_color(muted)
                                                .hover(|style| {
                                                    style.bg(theme.border).text_color(fg)
                                                })
                                                .text_xs()
                                                .child(Icon::new(IconName::Close).size_3())
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
                                ),
                        );
                        continue;
                    }
                }
                project_list = project_list.child(self.terminal_rows(&project.id, cx));
                project_list = project_list.child(self.agent_rows(&project.id, cx));
            }
        }
        sidebar =
            sidebar.child(project_list).child(
                div().px_3().py_2().border_t_1().border_color(border).child(
                    div()
                        .id("add-project")
                        .cursor_pointer()
                        .px_2()
                        .py_2()
                        .rounded_md()
                        .hover(|style| style.bg(active_bg))
                        .text_sm()
                        .child("+ Add Project…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_project(&AddProject, window, cx)
                        })),
                ),
            );
        let visible = self.visible_ids();
        let mut content = v_flex().flex_1().min_w_0().h_full();
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
                        .child(
                            div()
                                .id("close-side")
                                .cursor_pointer()
                                .child(Icon::new(IconName::Close).size_3())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.close_tab(side, window, cx)
                                })),
                        ),
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
                if event.keystroke.key == "escape" && this.process_project.is_some() {
                    this.cancel_process(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape"
                    && (this.rename.is_some()
                        || this.question_parent.is_some()
                        || this.customization_provider.is_some()
                        || this.customization_preview.is_some())
                {
                    this.question_parent = None;
                    this.rename = None;
                    this.customization_provider = None;
                    this.customization_preview = None;
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
        let sidebar_owner = cx.weak_entity();
        body = body.child(if self.perf_layout == "no-sidebar" {
            content.into_any_element()
        } else {
            h_resizable("workspace-sidebar-split")
                .with_state(&self.sidebar_state)
                .on_resize(move |state, _, cx| {
                    let width = state.read(cx).sizes().first().copied();
                    if let Some(width) = width {
                        let _ = sidebar_owner.update(cx, |this, _| {
                            this.sidebar_width = f32::from(width);
                            this.save_layout();
                        });
                    }
                })
                .child(
                    resizable_panel()
                        .size(px(self.sidebar_width))
                        .flex_none()
                        .size_range(px(220.)..px(520.))
                        .child(sidebar),
                )
                .child(
                    resizable_panel()
                        .size_range(px(320.)..Pixels::MAX)
                        .child(content),
                )
                .into_any_element()
        });
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
        if let Some(notice) = &self.customization_notice {
            body = body.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .p_3()
                    .bg(bg)
                    .child(
                        h_flex()
                            .gap_3()
                            .child(div().flex_1().text_sm().child(notice.clone()))
                            .child(
                                Button::new("dismiss-customization-notice")
                                    .small()
                                    .label("Dismiss")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.customization_notice = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if let Some(preview) = self.customization_preview.clone() {
            let proposal = preview.proposal;
            let reviewed = proposal.clone();
            body = body.child(div().absolute().inset_0().bg(hsla(0., 0., 0., 0.4))
                .flex().justify_center().pt(px(60.)).child(
                    v_flex().w(px(720.)).h(px(560.)).p_4().gap_3().rounded_lg().bg(bg)
                        .child(div().font_bold().child("Review DevDesk customization"))
                        .child(div().text_sm().child("Review the proposed configuration below, including commands and autostart. Appearance updates live. Running processes require a restart; autostart applies when a project is opened. Undo restores configuration, not command side effects."))
                        .child(v_flex().id("customization-proposal").flex_1().min_h_0().overflow_y_scroll().text_sm()
                            .children(preview.changes.lines().chain(std::iter::once("— Complete proposed configuration —")).chain(proposal.lines()).enumerate().map(|(index, line)| {
                                div().id(SharedString::from(format!("proposal-line-{index}"))).child(line.to_string())
                            })))
                        .child(h_flex().gap_3()
                            .child(Button::new("apply-customization").label("Apply changes")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.core.send(CoreCommand::ApplyCustomization(reviewed.clone()));
                                    this.customization_preview = None;
                                    this.restore_focus(window, cx);
                                    cx.notify();
                                })))
                            .child(Button::new("cancel-customization").label("Cancel")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.customization_preview = None;
                                    this.restore_focus(window, cx);
                                    cx.notify();
                                }))))));
        }
        if self.rename.is_some()
            || self.question_parent.is_some()
            || self.customization_provider.is_some()
        {
            body = body.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(hsla(0., 0., 0., 0.3))
                    .flex()
                    .justify_center()
                    .items_start()
                    .pt(px(100.))
                    .child(
                        v_flex()
                            .w(px(560.))
                            .h_auto()
                            .p_4()
                            .gap_3()
                            .rounded_lg()
                            .bg(bg)
                            .child(if self.customization_provider.is_some() {
                                "Customize DevDesk · describe your workspace"
                            } else if self.question_parent.is_some() {
                                "Ask Agent Question · read-only snapshot"
                            } else {
                                "Rename terminal"
                            })
                            .when(self.customization_provider.is_some(), |panel| panel.child(div().text_sm().child("Try: use warm-dark, set text to 16, and narrow the sidebar. The agent prepares a proposal; choose Review Customization in Cmd-P when ready.")))
                            .child(Input::new(&self.rename_input))
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(
                                        div()
                                            .id("rename-save")
                                            .cursor_pointer()
                                            .child(if self.customization_provider.is_some() {
                                                "Start conversation ↵"
                                            } else if self.question_parent.is_some() {
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
                                                this.customization_provider = None;
                                                this.question_parent = None;
                                                this.restore_focus(window, cx);
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    ),
            );
        }
        if self.process_project.is_some() {
            body = body.child(self.process_form(cx));
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
