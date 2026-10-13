//! UI mirror of the supervisor. All terminal entities remain alive across tab
//! and project switches; only selected entities are attached to the render tree.
mod agent_form;
mod agent_form_smoke;
mod customization_smoke;
mod meetings_view;
mod modal_focus_smoke;
mod notes_smoke;
mod notes_view;
mod process_form;
mod setup_view;
mod side_smoke;
mod sidebar_navigation;
mod sidebar_smoke;
mod skills_smoke;
mod skills_view;
use crate::terminal_view::TerminalView;
use chrono::{Local, TimeZone};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel};
use gpui_kit::component::{ActiveTheme, IconName, Sizable, Theme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use sidebar_navigation::SidebarRow;
pub(crate) use sidebar_navigation::key_bindings as sidebar_key_bindings;
use std::collections::{BTreeMap, BTreeSet};
use tervy_core::config::{Config, Project};
use tervy_core::supervisor::{
    Command as CoreCommand, CoreEvent, SessionId, SessionInfo, SessionStatus, SupervisorHandle,
};
use tervy_core::{agent, layout, meetings, notes, skills};

gpui_kit::actions!(
    tervy,
    [
        Palette,
        NewTerminal,
        NewNote,
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
        Tab9,
        FocusSidebar,
        ToggleSidebar,
        SidebarUp,
        SidebarDown,
        SidebarActivate,
        SidebarCollapse,
        SidebarExpand,
        SidebarExit
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
    NewNote,
    TerminalPreset(String, String),
    Close,
    Rename,
    Add,
    Setup,
    Start(SessionId),
    Stop(SessionId),
    Restart(SessionId),
    EditProcess(SessionId),
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
    ToggleSidebar,
    OpenUrl(String),
}

pub struct Workspace {
    focus: FocusHandle,
    sidebar_focus: FocusHandle,
    sidebar_cursor: Option<SidebarRow>,
    sidebar_scroll_pending: bool,
    sidebar_scroll: ScrollHandle,
    restoring: bool,
    last_layout: Option<layout::Layout>,
    approvals: usize,
    stress: Option<crate::m2_perf::ManyProbe>,
    core: SupervisorHandle,
    config: Config,
    projects: Vec<Project>,
    notes_open: bool,
    skills_open: bool,
    meetings_open: bool,
    notes_expanded: bool,
    skills_expanded: bool,
    skill_sources_expanded: BTreeSet<String>,
    skill_source_counts: BTreeMap<String, usize>,
    skill_items: Vec<skills::Summary>,
    skill_next_offset: Option<usize>,
    skill_searching: bool,
    skill_selected: Option<String>,
    skill_loaded: Option<skills::Skill>,
    skill_new: bool,
    skill_saving: bool,
    skill_error: Option<String>,
    skill_source: String,
    skill_search_input: Entity<InputState>,
    skill_name_input: Entity<InputState>,
    skill_content_input: Entity<TextareaState>,
    skill_file_path: Option<String>,
    skill_file_revision: Option<String>,
    skill_file_saved_content: String,
    skill_file_input: Entity<TextareaState>,
    skill_file_name_input: Entity<InputState>,
    skill_file_saving: bool,
    meetings_expanded: bool,
    meetings: Vec<meetings::Meeting>,
    meeting_selected: Option<String>,
    meeting_error: Option<String>,
    meeting_record_choice: bool,
    meeting_timer_running: bool,
    meeting_title_input: Entity<InputState>,
    meeting_notes_input: Entity<TextareaState>,
    meeting_notes_editing: bool,
    notes: Vec<notes::Summary>,
    note_next_offset: Option<usize>,
    note_searching: bool,
    note_selected: Option<String>,
    note_loaded: Option<notes::Note>,
    note_new: bool,
    note_saving: bool,
    note_error: Option<String>,
    note_search_input: Entity<InputState>,
    note_title_input: Entity<InputState>,
    note_body_input: Entity<TextareaState>,
    note_modal_open: bool,
    note_modal_saving: bool,
    note_modal_error: Option<String>,
    note_modal_title_input: Entity<InputState>,
    note_modal_body_input: Entity<TextareaState>,
    tabs: BTreeMap<SessionId, ShellTab>,
    memory: BTreeMap<SessionId, (tervy_core::supervisor::LaunchId, Option<u64>)>,
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
    setup_open: bool,
    readiness: Option<crate::setup::Readiness>,
    checking_setup: bool,
    update_available: Option<String>,
    update_error: Option<String>,
    update_install_pending: bool,
    palette_query: String,
    rename: Option<SessionId>,
    question_parent: Option<SessionId>,
    customization_provider: Option<agent::Provider>,
    customization_preview: Option<tervy_core::customization::Preview>,
    customization_notice: Option<String>,
    /// Only the UI owns split presentation; each PTY remains actor-owned.
    /// UI-owned divider state survives renders; actor storage receives only the
    /// final width on mouse-up, never every pointer movement.
    sidebar_state: Entity<ResizableState>,
    sidebar_width: f32,
    sidebar_collapsed: bool,
    split_states: BTreeMap<SessionId, Entity<ResizableState>>,
    side_panes: BTreeMap<SessionId, SessionId>,
    side_focus: Option<SessionId>,
    rename_input: Entity<InputState>,
    process_project: Option<String>,
    process_edit: Option<String>,
    process_autostart: bool,
    process_name_input: Entity<InputState>,
    process_command_input: Entity<InputState>,
    process_error: Option<String>,
    process_saving: bool,
    agent_launch: Option<agent_form::AgentLaunch>,
    agent_args_input: Entity<InputState>,
    agent_launch_error: Option<String>,
    dialog_pending: bool,
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
        let note_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search titles and text"));
        let note_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Note title"));
        let note_body_input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Write a note…"));
        let note_modal_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Note title"));
        let note_modal_body_input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Write a note…"));
        let skill_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search skills"));
        let skill_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("skill-name"));
        let skill_content_input = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(
                "---\nname: skill-name\ndescription: When to use this skill\n---\n\nInstructions",
            )
        });
        let skill_file_input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Supporting file text"));
        let skill_file_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("references/notes.md"));
        let meeting_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Meeting title"));
        let meeting_notes_input =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Meeting notes"));
        let note_search_subscription =
            cx.subscribe_in(&note_search_input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                    this.search_notes(cx);
                }
            });
        let note_title_subscription =
            cx.subscribe_in(&note_title_input, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let note_body_subscription =
            cx.subscribe_in(&note_body_input, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let skill_search_subscription =
            cx.subscribe_in(&skill_search_input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                    this.search_skills(cx);
                }
            });
        let skill_name_subscription =
            cx.subscribe_in(&skill_name_input, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let skill_content_subscription =
            cx.subscribe_in(&skill_content_input, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let skill_file_subscription =
            cx.subscribe_in(&skill_file_input, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let agent_args_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Optional command-line arguments"));
        let agent_args_subscription =
            cx.subscribe_in(&agent_args_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.launch_agent_from_form(window, cx);
                } else if matches!(event, InputEvent::Change) {
                    if this.agent_launch_error.is_some() {
                        this.agent_launch_error =
                            agent::parse_launch_args(&this.agent_args_input.read(cx).value()).err();
                    }
                    cx.notify();
                }
            });
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
            if std::env::var("TERVY_PERF_EXIT").as_deref() == Ok("1") {
                return;
            }
            let _ = weak.update(cx, |this, cx| {
                this.ready = true;
                if this.perf_layout.is_empty() {
                    this.check_setup(cx);
                }
                if this.perf_layout == "sidebar" {
                    if !this.projects.is_empty() {
                        this.smoke_sidebar(window, cx);
                    }
                    return;
                }
                if this.perf_layout == "setup" {
                    this.smoke_setup(window, cx);
                    return;
                }
                if this.perf_layout == "agent-launch" {
                    this.smoke_agent_form(window, cx);
                    return;
                }
                if this.perf_layout == "modal-focus" {
                    this.smoke_modal_focus(window, cx);
                    return;
                }
                if this.perf_layout == "skills" {
                    this.smoke_skills(window, cx);
                    return;
                }
                if this.perf_layout == "notes" {
                    this.smoke_notes(window, cx);
                    return;
                }
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
                            size: tervy_term::Dimensions::default(),
                        });
                    }
                    let timer = cx
                        .background_executor()
                        .timer(std::time::Duration::from_secs(5));
                    this._tasks.push(cx.spawn(async move |view, cx| {
                        timer.await;
                        let _ = view.update(cx, |this, cx| {
                            use tervy_term::GridDimensions as _;
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
                            eprintln!("tervy-idle-grids {:?}", rows);
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
            sidebar_focus: cx.focus_handle(),
            sidebar_cursor: None,
            sidebar_scroll_pending: false,
            sidebar_scroll: ScrollHandle::new(),
            restoring: std::env::var_os("TERVY_PERF_LAYOUT").is_none(),
            last_layout: None,
            approvals: 0,
            stress: (std::env::var("TERVY_PERF_LAYOUT").as_deref() == Ok("many"))
                .then(crate::m2_perf::ManyProbe::new),
            core,
            config: Config::default(),
            projects: vec![],
            notes_open: false,
            skills_open: false,
            meetings_open: false,
            notes_expanded: false,
            skills_expanded: false,
            skill_sources_expanded: BTreeSet::new(),
            skill_source_counts: BTreeMap::new(),
            skill_items: vec![],
            skill_next_offset: None,
            skill_searching: false,
            skill_selected: None,
            skill_loaded: None,
            skill_new: false,
            skill_saving: false,
            skill_error: None,
            skill_source: "claude".into(),
            skill_search_input,
            skill_name_input,
            skill_content_input,
            skill_file_path: None,
            skill_file_revision: None,
            skill_file_saved_content: String::new(),
            skill_file_input,
            skill_file_name_input,
            skill_file_saving: false,
            meetings_expanded: false,
            meetings: vec![],
            meeting_selected: None,
            meeting_error: None,
            meeting_record_choice: false,
            meeting_timer_running: false,
            meeting_title_input,
            meeting_notes_input,
            meeting_notes_editing: false,
            notes: vec![],
            note_next_offset: None,
            note_searching: false,
            note_selected: None,
            note_loaded: None,
            note_new: false,
            note_saving: false,
            note_error: None,
            note_search_input,
            note_title_input,
            note_body_input,
            note_modal_open: false,
            note_modal_saving: false,
            note_modal_error: None,
            note_modal_title_input,
            note_modal_body_input,
            tabs: BTreeMap::new(),
            memory: BTreeMap::new(),
            tab_order: vec![],
            active_project: None,
            selected: BTreeMap::new(),
            expanded: BTreeSet::new(),
            pending: BTreeSet::new(),
            ready: false,
            perf_layout: std::env::var("TERVY_PERF_LAYOUT").unwrap_or_default(),
            error: None,
            palette,
            palette_open: false,
            setup_open: std::env::var_os("TERVY_PERF_LAYOUT").is_none()
                && crate::setup::welcome_needed(&tervy_core::config::support_directory()),
            readiness: None,
            checking_setup: false,
            update_available: None,
            update_error: None,
            update_install_pending: false,
            palette_query: String::new(),
            rename: None,
            question_parent: None,
            customization_provider: None,
            customization_preview: None,
            customization_notice: None,
            sidebar_state: cx.new(|_| ResizableState::default()),
            sidebar_width: 300.,
            sidebar_collapsed: false,
            split_states: BTreeMap::new(),
            side_panes: BTreeMap::new(),
            side_focus: None,
            rename_input,
            process_project: None,
            process_edit: None,
            process_autostart: false,
            process_name_input,
            process_command_input,
            process_error: None,
            process_saving: false,
            agent_launch: None,
            agent_args_input,
            agent_launch_error: None,
            dialog_pending: false,
            shutdown: None,
            _tasks: vec![task],
            _subscriptions: vec![
                subscription,
                name_subscription,
                command_subscription,
                agent_args_subscription,
                note_search_subscription,
                note_title_subscription,
                note_body_subscription,
                skill_search_subscription,
                skill_name_subscription,
                skill_content_subscription,
                skill_file_subscription,
            ],
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
                | "sidebar"
                | "sidebar-running"
                | "skills"
                | "notes"
                | "agent-launch"
                | "modal-focus"
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
            CoreEvent::MeetingsListed(meetings) => {
                self.meetings = meetings;
                if self.meeting_selected.is_none()
                    && let Some(id) = self.meetings.first().map(|meeting| meeting.id.clone())
                {
                    self.select_meeting(id, window, cx);
                }
                cx.notify();
            }
            CoreEvent::MeetingUpdated(meeting) => {
                let previous = self
                    .meetings
                    .iter()
                    .find(|item| item.id == meeting.id)
                    .cloned();
                if self.meeting_selected.as_ref() == Some(&meeting.id) {
                    let old_notes = previous
                        .as_ref()
                        .map(|item| item.notes.as_str())
                        .unwrap_or("");
                    if self.meeting_notes_input.read(cx).value().as_ref() == old_notes {
                        self.meeting_notes_input.update(cx, |input, cx| {
                            input.set_value(meeting.notes.clone(), window, cx)
                        });
                    }
                    let old_title = previous
                        .as_ref()
                        .map(|item| item.title.as_str())
                        .unwrap_or("");
                    if self.meeting_title_input.read(cx).value().as_ref() == old_title {
                        self.meeting_title_input.update(cx, |input, cx| {
                            input.set_value(meeting.title.clone(), window, cx)
                        });
                    }
                }
                if let Some(item) = self.meetings.iter_mut().find(|item| item.id == meeting.id) {
                    *item = meeting.clone();
                } else {
                    self.meetings.insert(0, meeting.clone());
                }
                self.meetings
                    .sort_by_key(|item| std::cmp::Reverse(item.started_at));
                if self.meeting_selected.is_none()
                    || (previous.is_none() && meeting.status == meetings::Status::Starting)
                {
                    self.select_meeting(meeting.id.clone(), window, cx);
                }
                if meeting.status == meetings::Status::Recording {
                    self.start_meeting_timer(cx);
                }
                cx.notify();
            }
            CoreEvent::MeetingDeleted(id) => {
                self.meetings.retain(|meeting| meeting.id != id);
                if self.meeting_selected.as_ref() == Some(&id) {
                    self.meeting_selected = None;
                }
                cx.notify();
            }
            CoreEvent::MeetingError(error) => {
                self.meeting_error = Some(error);
                cx.notify();
            }
            CoreEvent::SkillsChanged => {
                self.refresh_skills(cx);
                if let Some(id) = self.skill_selected.clone() {
                    if self.skill_is_dirty(cx) || self.skill_file_is_dirty(cx) {
                        self.skill_error = Some(
                            "Skill files changed on disk. Save your draft elsewhere, then reload."
                                .into(),
                        );
                    } else {
                        self.core.send(CoreCommand::ReadSkill(id));
                    }
                }
            }
            CoreEvent::SkillsListed {
                query,
                offset,
                items,
                source_counts,
            } => {
                if query == self.skill_search_input.read(cx).value().as_ref() {
                    let length = items.len();
                    let total: usize = source_counts.values().sum();
                    if offset == 0 {
                        self.skill_source_counts = source_counts;
                        self.skill_items = items;
                        self.skill_searching = false;
                        self.skill_next_offset = (length < total).then_some(length);
                        if let Some(id) = self.skill_selected.as_ref()
                            && let Some(summary) =
                                self.skill_items.iter().find(|item| &item.id == id)
                            && self
                                .skill_loaded
                                .as_ref()
                                .is_some_and(|skill| skill.summary.revision != summary.revision)
                        {
                            if self.skill_is_dirty(cx) || self.skill_file_is_dirty(cx) {
                                self.skill_error = Some("Skill changed on disk. Save your draft elsewhere, then reload.".into());
                            } else {
                                self.core.send(CoreCommand::ReadSkill(id.clone()));
                            }
                        }
                    } else if offset == self.skill_items.len() {
                        self.skill_items.extend(items);
                        self.skill_next_offset =
                            (offset + length < total).then_some(offset + length);
                    }
                    cx.notify();
                }
            }
            CoreEvent::SkillRead { id, skill } => {
                if self.skill_selected.as_deref() == Some(&id) {
                    self.load_skill(skill, window, cx);
                }
            }
            CoreEvent::SkillFileRead {
                id,
                path,
                content,
                revision,
            } => {
                if self.skill_selected.as_deref() == Some(&id)
                    && self.skill_file_path.as_deref() == Some(&path)
                {
                    self.skill_file_saved_content = content.clone();
                    self.skill_file_revision = Some(revision);
                    self.skill_file_input
                        .update(cx, |input, cx| input.set_value(content, window, cx));
                    cx.notify();
                }
            }
            CoreEvent::SkillSaved { skill, from_ui } => {
                let selected = self.skill_selected.as_deref() == Some(&skill.summary.id);
                let load = (from_ui && self.skill_saving)
                    || (selected && !self.skill_is_dirty(cx) && !self.skill_file_is_dirty(cx));
                if from_ui {
                    self.skill_saving = false;
                }
                if load {
                    self.load_skill(skill, window, cx);
                } else if selected {
                    self.skill_error =
                        Some("Skill changed outside this editor. Reload before saving.".into());
                }
                self.refresh_skills(cx);
                cx.notify();
            }
            CoreEvent::SkillDeleted(id) => {
                if self.skill_selected.as_deref() == Some(&id) {
                    self.skill_selected = None;
                    self.skill_loaded = None;
                    self.skill_new = false;
                }
                self.refresh_skills(cx);
                cx.notify();
            }
            CoreEvent::SkillFileSaved { id, path, revision } => {
                if self.skill_selected.as_deref() == Some(&id)
                    && self.skill_file_path.as_deref() == Some(&path)
                {
                    self.skill_file_revision = Some(revision);
                    self.skill_file_saved_content =
                        self.skill_file_input.read(cx).value().to_string();
                    self.skill_file_saving = false;
                    self.skill_error = None;
                    self.core.send(CoreCommand::ReadSkill(id));
                    cx.notify();
                }
            }
            CoreEvent::SkillDeleteApproval {
                request_id,
                title,
                agent,
            } => {
                if self.dialog_pending {
                    self.core.send(CoreCommand::ResolveSkillDelete {
                        request_id,
                        approved: false,
                    });
                } else {
                    self.dialog_pending = true;
                    let answer = window.prompt(
                        PromptLevel::Warning,
                        "Delete skill?",
                        Some(&format!("{agent} requests deleting {title} and its files.")),
                        &["Cancel", "Delete skill"],
                        cx,
                    );
                    self._tasks.push(cx.spawn(async move |view, cx| {
                        let approved = answer.await == Ok(1);
                        let _ = view.update(cx, |this, cx| {
                            this.dialog_pending = false;
                            this.core.send(CoreCommand::ResolveSkillDelete {
                                request_id,
                                approved,
                            });
                            cx.notify();
                        });
                    }));
                }
            }
            CoreEvent::SkillError(error) => {
                self.skill_saving = false;
                self.skill_file_saving = false;
                self.skill_error = Some(error);
                cx.notify();
            }
            CoreEvent::NotesListed {
                query,
                offset,
                items,
            } => {
                if query == self.note_search_input.read(cx).value().as_ref() {
                    let length = items.len();
                    let accepted = if offset == 0 {
                        self.notes = items;
                        self.note_searching = false;
                        true
                    } else if offset == self.notes.len() {
                        self.notes.extend(items);
                        true
                    } else {
                        false
                    };
                    if accepted {
                        self.note_next_offset = (length == 100).then_some(offset + length);
                    }
                    cx.notify();
                }
            }
            CoreEvent::NoteRead { id, note } => {
                if self.note_selected.as_ref() != Some(&id) {
                    return;
                }
                if let Some(note) = note {
                    self.load_note(note, window, cx);
                } else {
                    self.note_error = Some("Note no longer exists".into());
                    cx.notify();
                }
            }
            CoreEvent::NoteSaved { note, from_ui } => {
                if from_ui && self.note_modal_saving {
                    self.note_modal_saving = false;
                    self.note_modal_open = false;
                    self.note_modal_error = None;
                    self.restore_focus(window, cx);
                }
                let should_load = (from_ui && self.note_saving)
                    || (self.note_selected.as_ref() == Some(&note.id) && !self.note_is_dirty(cx));
                if from_ui {
                    self.note_saving = false;
                }
                if should_load {
                    self.note_selected = Some(note.id.clone());
                    self.note_new = false;
                    self.load_note(note, window, cx);
                } else if self.note_selected.as_ref() == Some(&note.id) {
                    self.note_error = Some(
                        "This note changed in an agent. Save your draft elsewhere, then reload it."
                            .into(),
                    );
                }
                self.refresh_notes(cx);
                cx.notify();
            }
            CoreEvent::NoteDeleted(id) => {
                self.note_error = None;
                if self.note_selected.as_ref() == Some(&id) {
                    let keep_draft = self.note_is_dirty(cx);
                    self.note_selected = None;
                    self.note_loaded = None;
                    self.note_new = keep_draft;
                    if keep_draft {
                        self.note_error = Some(
                            "The saved note was deleted. You can save this draft as a new note."
                                .into(),
                        );
                    } else {
                        self.note_title_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        self.note_body_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                }
                self.refresh_notes(cx);
                cx.notify();
            }
            CoreEvent::NoteDeleteApproval {
                request_id,
                title,
                agent,
            } => {
                if self.dialog_pending {
                    self.core.send(CoreCommand::ResolveNoteDelete {
                        request_id,
                        approved: false,
                    });
                } else {
                    self.dialog_pending = true;
                    let answer = window.prompt(
                        PromptLevel::Warning,
                        "Delete saved note?",
                        Some(&format!("{agent} requested deletion of ‘{title}’.")),
                        &["Cancel", "Delete note"],
                        cx,
                    );
                    self._tasks.push(cx.spawn(async move |view, cx| {
                        let approved = answer.await == Ok(1);
                        let _ = view.update(cx, |this, cx| {
                            this.dialog_pending = false;
                            this.core.send(CoreCommand::ResolveNoteDelete {
                                request_id,
                                approved,
                            });
                            cx.notify();
                        });
                    }));
                }
            }
            CoreEvent::NoteError(error) => {
                if self.note_modal_saving {
                    self.note_modal_saving = false;
                    self.note_modal_error = Some(error);
                } else {
                    self.note_saving = false;
                    self.note_searching = false;
                    self.note_error = Some(error);
                }
                cx.notify();
            }
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
                        .unwrap_or(300.)
                        .clamp(220., 520.);
                    self.sidebar_state = cx.new(|_| ResizableState::default());
                    self.sidebar_collapsed = layout.sidebar_collapsed;
                    self.expanded = layout.expanded.into_iter().collect();
                    self.skills_expanded = layout.skills_expanded;
                    self.notes_expanded = layout.notes_expanded;
                    self.meetings_expanded = layout.meetings_expanded;
                    self.skill_sources_expanded =
                        layout.skill_sources_expanded.into_iter().collect();
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
                self.refresh_skills(cx);
                self.refresh_notes(cx);
                self.core.send(CoreCommand::ListMeetings);
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
                    "Processes from the previous Tervy run are still running",
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
                        .unwrap_or(300.)
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
                if self.ready && self.perf_layout == "sidebar" {
                    self.smoke_sidebar(window, cx);
                }
                if self.ready && self.perf_layout == "customization" {
                    self.smoke_customization(window, cx);
                }
            }
            CoreEvent::ConfigError(error) => self.error = Some(error),
            CoreEvent::ProcessAdded(project) => {
                if self.process_project.as_ref() == Some(&project) {
                    self.process_project = None;
                    self.process_edit = None;
                    self.process_error = None;
                    self.process_saving = false;
                    self.restore_focus(window, cx);
                }
            }
            CoreEvent::ProcessEdited { project, process } => {
                if self.process_project.as_ref() == Some(&project)
                    && self.process_edit.as_ref() == Some(&process)
                {
                    self.process_project = None;
                    self.process_edit = None;
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
                        "tervy-scrollback-session id={} status={:?}",
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
                    if stress.attached == tervy_core::perf::noisy_count() + 1 {
                        eprintln!("tervy-many-progress attached all");
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
                if self.dialog_pending {
                    return;
                }
                self.dialog_pending = true;
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
                        this.dialog_pending = false;
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
                        "tervy-quit-smoke PASS shortcut_requested_shutdown=true clean_shutdown=true"
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
                if self.update_install_pending && clean {
                    crate::updater::resume_install();
                } else {
                    cx.quit();
                }
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
        if self.dialog_pending {
            return;
        }
        let (name, detail) = match &target {
            RemovalTarget::Project(id) => {
                let Some(project) = self.projects.iter().find(|p| p.id == *id) else {
                    return;
                };
                (
                    project.name.clone(),
                    "Stop this project's sessions and remove it from Tervy. Files in its folder are kept.",
                )
            }
            RemovalTarget::Session(id) => {
                let Some(tab) = self.tabs.get(id) else {
                    return;
                };
                (
                    tab.info.name.clone(),
                    if tab.info.agent.is_some() {
                        "Stop this agent and remove its Tervy entry. The CLI's saved conversation is kept."
                    } else {
                        "Stop this process and remove its definition from Tervy's global config."
                    },
                )
            }
        };
        self.dialog_pending = true;
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
                this.dialog_pending = false;
                if answer == Ok(1) {
                    this.core.send(match target {
                        RemovalTarget::Project(id) => CoreCommand::RemoveProject(id),
                        RemovalTarget::Session(id) => CoreCommand::RemoveSession(id),
                    });
                }
            });
        }));
    }
    fn terminal_rows(&self, project: &str, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected_bg: Hsla = rgb(0x252b36).into();
        let hover_bg: Hsla = rgb(0x1f232b).into();
        let add_project = project.to_string();
        let mut rows = v_flex().min_w_0().child(
            h_flex()
                .group("sidebar-actions")
                .pl_8()
                .pr_2()
                .pt_2()
                .pb_1()
                .gap_2()
                .child(
                    div()
                        .size(px(20.))
                        .rounded_md()
                        .bg(rgb(0x2c323c))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_kit::component::Icon::new(IconName::SquareTerminal)
                                .size(px(13.))
                                .text_color(rgb(0xa3aab7)),
                        ),
                )
                .child(div().flex_1().text_sm().font_semibold().child("Terminals"))
                .child(
                    sidebar_actions(self.sidebar_focus.contains_focused(window, cx)).child(
                        Button::new(SharedString::from(format!("add-terminal-{project}")))
                            .icon(IconName::Plus)
                            .small()
                            .ghost()
                            .tooltip("New terminal")
                            .accessibility_label("New terminal")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.active_project = Some(add_project.clone());
                                this.new_terminal(&NewTerminal, window, cx);
                                this.update_visibility(window, cx);
                                this.save_layout();
                                cx.notify();
                            })),
                    ),
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
            rows = rows.child(
                h_flex()
                    .id(("terminal-row", id.0))
                    .group("sidebar-actions")
                    .relative()
                    .child(self.sidebar_indicator(SidebarRow::Session(id), window, cx))
                    .ml(px(42.))
                    .border_l_1()
                    .border_color(rgb(0x2c323c))
                    .px_2()
                    .h(px(30.))
                    .gap_1()
                    .min_w_0()
                    .rounded_md()
                    .when(self.active() == Some(id), |row| {
                        row.bg(selected_bg).font_medium()
                    })
                    .hover(|style| {
                        style.bg(if self.active() == Some(id) {
                            selected_bg
                        } else {
                            hover_bg
                        })
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .mr_1()
                            .child(status_indicator(&tab.info, theme))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.click_sidebar_row(SidebarRow::Session(id), window, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new(("shell", id.0))
                            .small()
                            .ghost()
                            .flex_1()
                            .min_w_0()
                            .px_0()
                            .accessibility_label(tab.info.name.clone())
                            .tooltip(tab.info.name.clone())
                            .child(
                                h_flex().w_full().min_w_0().child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .when(
                                            matches!(
                                                tab.info.status,
                                                SessionStatus::Stopped
                                                    | SessionStatus::Exited { .. }
                                            ),
                                            |label| label.text_color(rgb(0xa3aab7)),
                                        )
                                        .child(tab.info.name.clone()),
                                ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.click_sidebar_row(SidebarRow::Session(id), window, cx)
                            })),
                    )
                    .child(
                        sidebar_actions(
                            self.sidebar_focus.contains_focused(window, cx)
                                && self.sidebar_cursor == Some(SidebarRow::Session(id)),
                        )
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .px_1()
                        .bg(if self.active() == Some(id) {
                            selected_bg
                        } else {
                            hover_bg
                        })
                        .child(
                            session_action(("close-terminal", id.0), "Close terminal")
                                .icon(IconName::Close)
                                .accessibility_label(format!("Close {}", tab.info.name))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.close_tab(id, window, cx)
                                })),
                        ),
                    ),
            );
        }
        if !self.tabs.values().any(|tab| {
            tab.info.project == project
                && tab.info.process.is_none()
                && tab.info.agent.is_none()
                && tab.open
        }) {
            rows = rows.child(
                div()
                    .pl_6()
                    .py_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No open terminals"),
            );
        }
        rows.into_any_element()
    }

    fn agent_rows(&self, project: &str, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected_bg: Hsla = rgb(0x252b36).into();
        let hover_bg: Hsla = rgb(0x1f232b).into();
        let launch_project = project.to_string();
        let workspace = cx.weak_entity();
        let mut rows = v_flex().min_w_0().child(
            h_flex()
                .group("sidebar-actions")
                .pl_8()
                .pr_2()
                .pt_2()
                .pb_1()
                .gap_2()
                .child(
                    div()
                        .size(px(20.))
                        .rounded_md()
                        .bg(rgb(0x2c323c))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_kit::component::Icon::default()
                                .data(include_bytes!("../../../assets/icons/sparkles.svg"))
                                .size(px(13.))
                                .text_color(rgb(0xa3aab7)),
                        ),
                )
                .child(div().flex_1().text_sm().font_semibold().child("Agents"))
                .child(
                    sidebar_actions(self.sidebar_focus.contains_focused(window, cx)).child(
                        Button::new(SharedString::from(format!("add-agent-{project}")))
                            .icon(IconName::Plus)
                            .small()
                            .ghost()
                            .tooltip("Launch agent")
                            .accessibility_label("Launch agent")
                            .dropdown_menu(move |mut menu, _, cx| {
                                let Some(view) = workspace.upgrade() else {
                                    return menu;
                                };
                                let readiness = &view.read(cx).readiness;
                                let providers = readiness
                                    .as_ref()
                                    .map(crate::setup::Readiness::installed_agents)
                                    .unwrap_or_default();
                                if providers.is_empty() {
                                    menu = menu.item(
                                        PopupMenuItem::new(if readiness.is_none() {
                                            "Checking installed agents…"
                                        } else {
                                            "No supported agents found"
                                        })
                                        .disabled(true),
                                    );
                                }
                                for provider in providers {
                                    let workspace = workspace.clone();
                                    let project = launch_project.clone();
                                    menu =
                                        menu.item(
                                            PopupMenuItem::new(format!(
                                                "Launch {} · {}",
                                                provider.label(),
                                                if provider.tracks_sessions() {
                                                    "resume + side questions"
                                                } else {
                                                    "basic terminal"
                                                }
                                            ))
                                            .on_click(move |_, window, cx| {
                                                let _ = workspace.update(cx, |this, cx| {
                                                    if this.shutdown.is_some() {
                                                        return;
                                                    }
                                                    this.show_agent_form(
                                                        project.clone(),
                                                        provider,
                                                        false,
                                                        None,
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }),
                                        );
                                }
                                let workspace = workspace.clone();
                                menu.separator().item(
                                    PopupMenuItem::new("Setup and Recheck Agents…").on_click(
                                        move |_, window, cx| {
                                            let _ = workspace.update(cx, |this, cx| {
                                                this.setup_open = true;
                                                this.focus.focus(window, cx);
                                                this.check_setup(cx);
                                            });
                                        },
                                    ),
                                )
                            }),
                    ),
                ),
        );
        for id in self.ordered_agent_ids() {
            let tab = &self.tabs[&id];
            if tab.info.project != project {
                continue;
            }
            let Some(agent) = &tab.info.agent else {
                continue;
            };
            let running = matches!(
                tab.info.status,
                SessionStatus::Starting | SessionStatus::Running | SessionStatus::Stopping
            );
            let status = if !running && agent.record.provider.tracks_sessions() {
                "Paused"
            } else if !running {
                "Stopped"
            } else if matches!(tab.info.status, SessionStatus::Stopping) {
                "Stopping"
            } else {
                match agent.status {
                    agent::Status::Launching => "Launching",
                    agent::Status::Running => "Running",
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
                },
            );
            rows = rows.child(
                // GPUI retains hover state only for identified elements. Keep
                // it on the entire row so padding and controls repaint too.
                h_flex()
                    .id(("agent-row", id.0))
                    .group("sidebar-actions")
                    .relative()
                    .child(self.sidebar_indicator(SidebarRow::Session(id), window, cx))
                    .ml(px(42.))
                    .border_l_1()
                    .border_color(rgb(0x2c323c))
                    .px_2()
                    .h(px(30.))
                    .min_w_0()
                    .rounded_md()
                    .when(is_side(&tab.info), |row| row.ml(px(54.)))
                    .when(
                        self.active() == Some(id) || self.side_focus == Some(id),
                        |row| row.bg(selected_bg),
                    )
                    .hover(|style| {
                        style.bg(
                            if self.active() == Some(id) || self.side_focus == Some(id) {
                                selected_bg
                            } else {
                                hover_bg
                            },
                        )
                    })
                    .gap_1()
                    .child(
                        div()
                            .flex_shrink_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .mr_1()
                            .child(status_indicator(&tab.info, theme))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.click_sidebar_row(SidebarRow::Session(id), window, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new(("agent", id.0))
                            .small()
                            .ghost()
                            .flex_1()
                            .min_w_0()
                            .px_0()
                            .accessibility_label(full_name.clone())
                            .tooltip(full_name)
                            .child(
                                h_flex().w_full().min_w_0().child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .when(!running, |label| label.text_color(rgb(0xa3aab7)))
                                        .child(tab.info.name.clone()),
                                ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.click_sidebar_row(SidebarRow::Session(id), window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .font_family("IBM Plex Mono")
                            .text_color(rgb(0xa3aab7))
                            .child(status.to_lowercase()),
                    )
                    .child(
                        sidebar_actions(
                            self.sidebar_focus.contains_focused(window, cx)
                                && self.sidebar_cursor == Some(SidebarRow::Session(id)),
                        )
                        .child(
                            session_action(("remove-agent", id.0), "Remove agent")
                                .icon(
                                    gpui_kit::component::Icon::default()
                                        .data(include_bytes!("../../../assets/icons/trash.svg")),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.confirm_remove(RemovalTarget::Session(id), window, cx);
                                })),
                        ),
                    )
                    .child(
                        session_action(
                            ("agent-toggle", id.0),
                            if running {
                                "Stop agent"
                            } else if agent.record.provider.tracks_sessions() {
                                "Resume agent"
                            } else {
                                "Restart agent"
                            },
                        )
                        .icon(if running {
                            IconName::Square
                        } else if agent.record.provider.tracks_sessions() {
                            IconName::Play
                        } else {
                            IconName::RotateCw
                        })
                        .accessibility_label(format!(
                            "{} {}",
                            if running {
                                "Stop"
                            } else if agent.record.provider.tracks_sessions() {
                                "Resume"
                            } else {
                                "Restart"
                            },
                            tab.info.name
                        ))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.choose_tab(id, window, cx);
                                if running {
                                    this.core.send(CoreCommand::Stop(id));
                                } else {
                                    this.show_resume_agent_form(id, window, cx);
                                }
                            },
                        )),
                    ),
            );
        }
        if !self
            .tabs
            .values()
            .any(|tab| tab.info.project == project && tab.info.agent.is_some())
        {
            rows = rows.child(
                div()
                    .pl_6()
                    .py_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No agents yet"),
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
            sidebar_collapsed: self.sidebar_collapsed,
            expanded: self.expanded.iter().cloned().collect(),
            skills_expanded: self.skills_expanded,
            notes_expanded: self.notes_expanded,
            meetings_expanded: self.meetings_expanded,
            skill_sources_expanded: self.skill_sources_expanded.iter().cloned().collect(),
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
        if self.skills_open || self.notes_open || self.meetings_open {
            return vec![];
        }
        let mut ids = self.active().into_iter().collect::<Vec<_>>();
        if let Some(side) = self.active().and_then(|id| self.side_panes.get(&id)) {
            ids.push(*side);
        }
        if self.stress.is_some() {
            for id in [
                SessionId(tervy_core::perf::noisy_count() as u64 + 1),
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
        for index in 0..tervy_core::perf::noisy_count() + 1 {
            self.core.send(CoreCommand::NewShell {
                project: ["probe", "probe2", "probe3"][index % 3].into(),
                size: tervy_term::Dimensions::default(),
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
            eprintln!("tervy-many-progress requesting shutdown");
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
                .get(&SessionId(tervy_core::perf::noisy_count() as u64 + 1))
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
                eprintln!("tervy-many-progress dispatching {marker}");
                dispatch = Some((focus, marker));
            }
        }
        self.schedule_stress(window, cx);
        dispatch
    }
    fn update_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sidebar_focused = self.sidebar_focus.contains_focused(window, cx);
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
        if sidebar_focused && !self.setup_open {
            self.sidebar_focus.focus(window, cx);
        } else {
            self.restore_focus(window, cx);
        }
    }

    fn open_project(&mut self, project: String, window: &mut Window, cx: &mut Context<Self>) {
        self.notes_open = false;
        self.skills_open = false;
        self.meetings_open = false;
        if self.sidebar_focus.contains_focused(window, cx) {
            self.focus.focus(window, cx);
        }
        let started = std::env::var_os("TERVY_PERF_SWITCH").map(|_| std::time::Instant::now());
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
                size: tervy_term::Dimensions::default(),
            });
        }
        self.update_visibility(window, cx);
        self.save_layout();
        cx.notify();
        if let Some(started) = started {
            window.on_next_frame(move |_, _| {
                eprintln!(
                    "tervy-switch frame_ms={:.3}",
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
        self.notes_open = false;
        self.skills_open = false;
        self.meetings_open = false;
        if self.sidebar_focus.contains_focused(window, cx) {
            self.focus.focus(window, cx);
        }
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
    fn new_terminal(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_focus.contains_focused(window, cx) {
            self.focus.focus(window, cx);
        }
        if self.shutdown.is_none()
            && let Some(project) = &self.active_project
        {
            self.core.send(CoreCommand::NewShell {
                project: project.clone(),
                size: tervy_term::Dimensions::default(),
            });
        }
    }
    fn close_terminal(&mut self, _: &CloseTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.setup_open {
            self.dismiss_setup(window, cx);
            return;
        }
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
    fn add_project(&mut self, _: &AddProject, window: &mut Window, cx: &mut Context<Self>) {
        if self.setup_open {
            self.dismiss_setup(window, cx);
        }
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
            Some(agent) if !agent.record.provider.tracks_sessions() => {
                Some("Read-only side questions are available for Claude and Codex agents")
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
                self.error = Some("Describe how you want to customize Tervy".into());
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
        if self.note_modal_open {
            self.note_modal_title_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if self.agent_launch.is_some() {
            self.agent_args_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if self.setup_open {
            self.focus.focus(window, cx);
            return;
        }
        if self.sidebar_focus.contains_focused(window, cx) {
            self.sidebar_focus.focus(window, cx);
            return;
        }
        if self.skills_open {
            self.skill_search_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if self.notes_open {
            self.note_search_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
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
        self.agent_launch = None;
        self.agent_launch_error = None;
        if self.setup_open {
            self.dismiss_setup(window, cx);
        }
        self.palette_open = true;
        self.palette_query.clear();
        self.palette.update(cx, |state, cx| {
            state.set_query("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
    fn palette_actions(&self, cx: &App) -> Vec<(String, PaletteAction)> {
        let installed_agents = self
            .readiness
            .as_ref()
            .map(crate::setup::Readiness::installed_agents)
            .unwrap_or_default();
        let mut actions = vec![
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
            (
                if self.sidebar_collapsed {
                    "Show Sidebar"
                } else {
                    "Hide Sidebar"
                }
                .into(),
                PaletteAction::ToggleSidebar,
            ),
            ("New Terminal".into(), PaletteAction::New),
            ("New Note…".into(), PaletteAction::NewNote),
            ("Close Terminal".into(), PaletteAction::Close),
            ("Rename Terminal".into(), PaletteAction::Rename),
            ("Add Project…".into(), PaletteAction::Add),
            ("Ask Agent Question…".into(), PaletteAction::Ask),
        ];
        for provider in installed_agents
            .iter()
            .filter(|provider| provider.tracks_sessions())
        {
            actions.push((
                format!("Customize Tervy with {}…", provider.label()),
                PaletteAction::Customize(*provider),
            ));
        }
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
            let default_provider = provider
                .and_then(agent::Provider::from_command)
                .unwrap_or_default();
            if installed_agents.contains(&default_provider) {
                actions.push((
                    "Launch Default Agent".into(),
                    PaletteAction::LaunchAgent(default_provider, false),
                ));
            }
            for provider in &installed_agents {
                actions.push((
                    format!(
                        "Launch {} · {}",
                        provider.label(),
                        if provider.tracks_sessions() {
                            "integrated"
                        } else {
                            "basic terminal"
                        }
                    ),
                    PaletteAction::LaunchAgent(*provider, false),
                ));
                if provider.tracks_sessions() {
                    actions.push((
                        format!("Resume other {} session…", provider.label()),
                        PaletteAction::LaunchAgent(*provider, true),
                    ));
                }
            }
            for (id, tab) in &self.tabs {
                if tab.info.project == *project && tab.info.agent.is_some() {
                    actions.push((
                        format!(
                            "{} {}",
                            if tab
                                .info
                                .agent
                                .as_ref()
                                .is_some_and(|agent| agent.record.provider.tracks_sessions())
                            {
                                "Resume"
                            } else {
                                "Restart"
                            },
                            tab.info.name
                        ),
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
                        format!("Edit {} process", tab.info.name),
                        PaletteAction::EditProcess(*id),
                    ));
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
        actions.push(("Setup and Prerequisites".into(), PaletteAction::Setup));
        actions.push(("Reload Configuration".into(), PaletteAction::Reload));
        actions.push(("Quit Tervy".into(), PaletteAction::Quit));
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
            PaletteAction::NewNote => self.show_new_note_modal(&NewNote, window, cx),
            PaletteAction::TerminalPreset(project, name) => self
                .core
                .send(CoreCommand::NewTerminalPreset { project, name }),
            PaletteAction::Close => self.close_terminal(&CloseTerminal, window, cx),
            PaletteAction::Rename => self.rename_terminal(&RenameTerminal, window, cx),
            PaletteAction::Add => self.add_project(&AddProject, window, cx),
            PaletteAction::Setup => {
                self.setup_open = true;
                self.focus.focus(window, cx);
                self.check_setup(cx);
            }
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
            PaletteAction::ToggleSidebar => self.toggle_sidebar(&ToggleSidebar, window, cx),
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
            PaletteAction::EditProcess(id) => self.show_edit_process_form(id, window, cx),
            PaletteAction::StartAll(project) => self.core.send(CoreCommand::StartAll(project)),
            PaletteAction::StopAll(project) => self.core.send(CoreCommand::StopAll(project)),
            PaletteAction::Recover => self.core.send(CoreCommand::ShowOrphans),
            PaletteAction::LaunchAgent(provider, picker) => {
                if let Some(project) = self.active_project.clone() {
                    self.show_agent_form(project, provider, picker, None, window, cx);
                }
            }
            PaletteAction::ResumeAgent(id) => {
                self.choose_tab(id, window, cx);
                self.show_resume_agent_form(id, window, cx);
            }
        }
        if !self.setup_open
            && self.rename.is_none()
            && self.question_parent.is_none()
            && self.customization_provider.is_none()
            && self.agent_launch.is_none()
        {
            self.restore_focus(window, cx);
        }
        cx.notify();
    }
    pub fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shutdown.is_some() || self.dialog_pending {
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
            .chain(
                self.active_recording()
                    .map(|_| "Meeting recording".to_string()),
            )
            .collect();
        if running.is_empty() {
            self.shutdown = Some(0);
            self.core.send(CoreCommand::Shutdown);
            cx.notify();
            return;
        }
        self.dialog_pending = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!(
                "Stop {} sessions and {}?",
                running.len(),
                if self.update_install_pending {
                    "update"
                } else {
                    "quit"
                }
            ),
            Some(&running.join("\n")),
            if self.update_install_pending {
                &["Later", "Stop all and update"]
            } else {
                &["Cancel", "Stop all and quit"]
            },
            cx,
        );
        self._tasks.push(cx.spawn(async move |view, cx| {
            let answer = answer.await;
            let _ = view.update(cx, |this, cx| {
                this.dialog_pending = false;
                if answer == Ok(1) {
                    this.shutdown = Some(this.tabs.len());
                    this.core.send(CoreCommand::Shutdown);
                    cx.notify();
                }
            });
        }));
    }

    pub(crate) fn update_event(
        &mut self,
        event: crate::updater::UpdateEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            crate::updater::UpdateEvent::Available(version) => {
                self.update_available = Some(version);
                self.update_error = None;
            }
            crate::updater::UpdateEvent::Current => {
                self.update_available = None;
                self.update_error = None;
            }
            crate::updater::UpdateEvent::Error(error) => self.update_error = Some(error),
            crate::updater::UpdateEvent::InstallRequested => {
                self.update_install_pending = true;
                self.request_quit(window, cx);
            }
        }
        cx.notify();
    }
}

/// Focus the editor when any part of its field is clicked, including frame padding.
fn modal_input(input: &Entity<InputState>, label: &'static str) -> Div {
    let editor = input.clone();
    div()
        .w_full()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            editor.update(cx, |input, cx| input.focus(window, cx));
        })
        .child(Input::new(input).aria_label(label))
}

/// Shared, restrained dialog surface; the overlay supplies viewport padding.
fn dialog_panel(width: f32, theme: &Theme) -> Div {
    v_flex()
        .w(px(width))
        .max_w_full()
        .min_h_0()
        .p_6()
        .gap_4()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
}

fn session_action(id: impl Into<ElementId>, label: &'static str) -> Button {
    Button::new(id)
        .small()
        .ghost()
        .tooltip(label)
        .accessibility_label(label)
}

/// Reveal controls with the row's hover group or keyboard focus.
fn sidebar_actions(keyboard_selected: bool) -> Div {
    h_flex()
        .invisible()
        .group_hover("sidebar-actions", |style| style.visible())
        .when(keyboard_selected, |actions| actions.visible())
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
/// Lifecycle takes precedence over agent activity, so an exited or stopping
/// agent cannot look ready because its last hook reported Idle or Working.
/// Render this beside sidebar buttons; their label content clips the glow.
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
            Some(agent::Status::Working | agent::Status::ActiveInferred) => rgb(0x82a8f5).into(),
            Some(agent::Status::Launching) => theme.warning,
            Some(agent::Status::Exited) => theme.muted_foreground,
            Some(agent::Status::Idle | agent::Status::Running) | None => rgb(0x3fcf8e).into(),
        },
    };
    let stopped = matches!(
        info.status,
        SessionStatus::Stopped | SessionStatus::Exited { .. }
    );
    let mut glow: Hsla = rgb(0x3fcf8e).into();
    glow.a = 0.35;
    div()
        .flex_shrink_0()
        .size(px(8.))
        .rounded_full()
        .when(stopped, |dot| dot.border_1().border_color(rgb(0x7d8594)))
        .when(!stopped, |dot| dot.bg(color))
        .when(
            matches!(info.status, SessionStatus::Running) && info.agent.is_none(),
            |dot| {
                dot.shadow(vec![BoxShadow {
                    color: glow,
                    blur_radius: px(5.),
                    spread_radius: px(1.),
                    offset: point(px(0.), px(0.)),
                    inset: false,
                }])
            },
        )
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let active_bg: Hsla = rgb(0x252b36).into();
        let sidebar_bg = rgb(0x16191f);
        let sidebar_panel: Hsla = rgb(0x1b1f26).into();
        let sidebar_hover: Hsla = rgb(0x1f232b).into();
        let sidebar_border = rgb(0x2c323c);
        let sidebar_text = rgb(0xe4e7ec);
        let sidebar_muted = rgb(0xa3aab7);
        if self.perf_layout == "minimal" {
            return div().size_full().bg(bg).into_any_element();
        }
        self.sync_sidebar_cursor();
        let mut sidebar = v_flex()
            .id("projects-sidebar")
            .track_focus(&self.sidebar_focus)
            .key_context("Sidebar")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.focus_sidebar(&FocusSidebar, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &SidebarUp, window, cx| this.move_sidebar(-1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SidebarDown, window, cx| this.move_sidebar(1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &SidebarActivate, window, cx| {
                this.activate_sidebar(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SidebarCollapse, window, cx| {
                this.expand_sidebar(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SidebarExpand, window, cx| {
                this.expand_sidebar(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &SidebarExit, window, cx| this.exit_sidebar(window, cx)),
            )
            .size_full()
            .min_w_0()
            .bg(sidebar_bg)
            .text_color(sidebar_text)
            .font_family("IBM Plex Sans")
            .child(
                v_flex()
                    .px_3()
                    .pt_4()
                    .pb_3()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(30.))
                                    .rounded_md()
                                    .bg(rgb(0x29364c))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        gpui_kit::component::Icon::default()
                                            .data(include_bytes!(
                                                "../../../assets/brand/mark-ui.svg"
                                            ))
                                            .size(px(20.))
                                            .text_color(rgb(0x82a8f5)),
                                    ),
                            )
                            .child(div().flex_1().text_base().font_semibold().child("Tervy"))
                            .child(
                                session_action("collapse-sidebar-header", "Collapse sidebar")
                                    .icon(IconName::PanelLeftClose)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_sidebar(&ToggleSidebar, window, cx)
                                    })),
                            ),
                    )
                    .child(
                        Button::new("sidebar-command-search")
                            .small()
                            .w_full()
                            .bg(sidebar_panel)
                            .border_1()
                            .border_color(sidebar_border)
                            .rounded_md()
                            .accessibility_label("Search or run a command")
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(
                                        gpui_kit::component::Icon::new(IconName::Search)
                                            .size(px(14.)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_left()
                                            .text_color(sidebar_muted)
                                            .child("Search or run a command"),
                                    )
                                    .child(
                                        div()
                                            .font_family("IBM Plex Mono")
                                            .text_xs()
                                            .text_color(sidebar_muted)
                                            .child("⌘K"),
                                    ),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_palette(&Palette, window, cx)
                            })),
                    ),
            );
        let skills_overview = self.skills_open && self.skill_selected.is_none() && !self.skill_new;
        let notes_overview = self.notes_open && self.note_selected.is_none() && !self.note_new;
        let mut project_list = v_flex()
            .id("sidebar-scroll")
            .track_scroll(&self.sidebar_scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_3()
            .gap_1();
        project_list = project_list.child(
            h_flex()
                .group("sidebar-actions")
                .px_2()
                .pt_2()
                .pb_2()
                .justify_between()
                .child(
                    div()
                        .id("projects-heading")
                        .text_size(px(11.))
                        .font_semibold()
                        .text_color(sidebar_muted)
                        .aria_label("Projects")
                        .child("P R O J E C T S"),
                )
                .child(
                    session_action("add-project", "Add project · ⌘O")
                        .icon(IconName::Plus)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_project(&AddProject, window, cx)
                        })),
                ),
        );
        for project in &self.projects {
            let id = project.id.clone();
            let toggle_id = id.clone();
            let remove_id = id.clone();
            let full_name = project.name.clone();
            let expanded = self.expanded.contains(&id);
            let selected = self.active_project.as_ref() == Some(&id);
            let process_total = self
                .tabs
                .values()
                .filter(|tab| {
                    tab.info.project == id && tab.info.process.is_some() && tab.info.agent.is_none()
                })
                .count();
            let process_running = self
                .tabs
                .values()
                .filter(|tab| {
                    tab.info.project == id
                        && tab.info.process.is_some()
                        && tab.info.agent.is_none()
                        && matches!(
                            tab.info.status,
                            SessionStatus::Running | SessionStatus::Starting
                        )
                })
                .count();
            let terminal_count = self
                .tabs
                .values()
                .filter(|tab| {
                    tab.info.project == id
                        && tab.info.process.is_none()
                        && tab.info.agent.is_none()
                        && tab.open
                })
                .count();
            let agent_count = self
                .tabs
                .values()
                .filter(|tab| tab.info.project == id && tab.info.agent.is_some())
                .count();
            let project_tint = [0x29394f, 0x44364f, 0x32483b, 0x4c3828, 0x2e4552][project
                .name
                .bytes()
                .fold(0usize, |n, byte| n.wrapping_add(byte as usize))
                % 5];
            let project_add_workspace = cx.weak_entity();
            let project_add_id = project.id.clone();
            let project_menu_workspace = cx.weak_entity();
            let mut project_panel = v_flex().min_w_0().rounded_lg().when(selected, |panel| {
                panel
                    .bg(sidebar_panel)
                    .border_1()
                    .border_color(sidebar_border)
            });
            project_panel = project_panel.child(
                h_flex()
                    .id(SharedString::from(format!("project-row-{id}")))
                    .aria_expanded(expanded)
                    .group("sidebar-actions")
                    .relative()
                    .child(self.sidebar_indicator(SidebarRow::Project(id.clone()), window, cx))
                    .hover(|style| style.bg(if selected { active_bg } else { sidebar_hover }))
                    .min_w_0()
                    .flex_shrink_0()
                    .px_2()
                    .h(px(36.))
                    .rounded_md()
                    .gap_1()
                    .child(
                        session_action(
                            SharedString::from(format!("expand-{id}")),
                            "Expand or collapse project",
                        )
                        .icon(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .accessibility_label(format!(
                            "{} {}",
                            if expanded { "Collapse" } else { "Expand" },
                            project.name
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&toggle_id) {
                                this.expanded.insert(toggle_id.clone());
                            }
                            this.save_layout();
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("project-{id}")))
                            .small()
                            .ghost()
                            .flex_1()
                            .min_w_0()
                            .px_0()
                            .tooltip(full_name)
                            .accessibility_label(project.name.clone())
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size(px(20.))
                                            .flex_shrink_0()
                                            .rounded_md()
                                            .bg(rgb(project_tint))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_xs()
                                            .font_semibold()
                                            .child(
                                                project
                                                    .name
                                                    .chars()
                                                    .next()
                                                    .unwrap_or('P')
                                                    .to_uppercase()
                                                    .to_string(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .font_semibold()
                                            .child(project.name.clone()),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.click_sidebar_row(SidebarRow::Project(id.clone()), window, cx)
                            })),
                    )
                    .child(
                        sidebar_actions(
                            self.sidebar_focus.contains_focused(window, cx)
                                && self.sidebar_cursor
                                    == Some(SidebarRow::Project(project.id.clone())),
                        )
                        .child(
                            session_action(
                                SharedString::from(format!("project-add-{remove_id}")),
                                "Add to project",
                            )
                            .icon(IconName::Plus)
                            .dropdown_menu(move |mut menu, _, cx| {
                                let process_workspace = project_add_workspace.clone();
                                let process_project = project_add_id.clone();
                                menu = menu.item(PopupMenuItem::new("Add process").on_click(
                                    move |_, window, cx| {
                                        let _ = process_workspace.update(cx, |this, cx| {
                                            this.show_process_form(
                                                process_project.clone(),
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                ));
                                let terminal_workspace = project_add_workspace.clone();
                                let terminal_project = project_add_id.clone();
                                menu = menu.item(PopupMenuItem::new("New terminal").on_click(
                                    move |_, window, cx| {
                                        let _ = terminal_workspace.update(cx, |this, cx| {
                                            this.active_project = Some(terminal_project.clone());
                                            this.new_terminal(&NewTerminal, window, cx);
                                            this.update_visibility(window, cx);
                                            this.save_layout();
                                        });
                                    },
                                ));
                                let providers = project_add_workspace
                                    .upgrade()
                                    .and_then(|view| {
                                        view.read(cx)
                                            .readiness
                                            .as_ref()
                                            .map(crate::setup::Readiness::installed_agents)
                                    })
                                    .unwrap_or_default();
                                for provider in providers {
                                    let agent_workspace = project_add_workspace.clone();
                                    let agent_project = project_add_id.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(format!("Launch {}", provider.label()))
                                            .on_click(move |_, window, cx| {
                                                let _ = agent_workspace.update(cx, |this, cx| {
                                                    this.show_agent_form(
                                                        agent_project.clone(),
                                                        provider,
                                                        false,
                                                        None,
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                        )
                        .child(
                            session_action(
                                SharedString::from(format!("remove-project-{remove_id}")),
                                "Project options",
                            )
                            .icon(IconName::Ellipsis)
                            .dropdown_menu(move |menu, _, _| {
                                let workspace = project_menu_workspace.clone();
                                let project = remove_id.clone();
                                menu.item(PopupMenuItem::new("Remove project").on_click(
                                    move |_, window, cx| {
                                        let _ = workspace.update(cx, |this, cx| {
                                            this.confirm_remove(
                                                RemovalTarget::Project(project.clone()),
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                ))
                            }),
                        ),
                    ),
            );
            if expanded {
                if process_total > 0 {
                    let add_process_project = project.id.clone();
                    let all_process_project = project.id.clone();
                    project_panel = project_panel.child(
                        h_flex()
                            .group("sidebar-actions")
                            .pl_8()
                            .pr_2()
                            .pt_2()
                            .pb_1()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(20.))
                                    .rounded_md()
                                    .bg(rgb(0x2c323c))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        gpui_kit::component::Icon::default()
                                            .data(include_bytes!(
                                                "../../../assets/icons/layers.svg"
                                            ))
                                            .size(px(13.))
                                            .text_color(rgb(0xa3aab7)),
                                    ),
                            )
                            .child(div().flex_1().text_sm().font_semibold().child("Processes"))
                            .child(
                                sidebar_actions(self.sidebar_focus.contains_focused(window, cx))
                                    .child(
                                        session_action(
                                            SharedString::from(format!(
                                                "all-processes-{}",
                                                project.id
                                            )),
                                            if process_running > 0 {
                                                "Stop all processes"
                                            } else {
                                                "Start all processes"
                                            },
                                        )
                                        .icon(if process_running > 0 {
                                            IconName::Square
                                        } else {
                                            IconName::Play
                                        })
                                        .on_click(
                                            cx.listener(move |this, _, _, _| {
                                                this.core.send(if process_running > 0 {
                                                    CoreCommand::StopAll(
                                                        all_process_project.clone(),
                                                    )
                                                } else {
                                                    CoreCommand::StartAll(
                                                        all_process_project.clone(),
                                                    )
                                                })
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "add-process-{}",
                                            project.id
                                        )))
                                        .icon(IconName::Plus)
                                        .small()
                                        .ghost()
                                        .tooltip("Add process")
                                        .accessibility_label("Add process")
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.show_process_form(
                                                    add_process_project.clone(),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                    ),
                            ),
                    );
                    let mut process_rows = v_flex()
                        .ml(px(42.))
                        .border_l_1()
                        .border_color(sidebar_border)
                        .min_w_0();
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
                            let menu_workspace = cx.weak_entity();
                            process_rows = process_rows.child(
                            v_flex()
                                .id(("process-row", id.0))
                                .group("sidebar-actions")
                                .relative()
                                .child(self.sidebar_indicator(SidebarRow::Session(id), window, cx))
                                .flex_shrink_0()
                                .ml_2()
                                .px_2()
                                .h(px(30.))
                                .rounded_md()
                                .min_w_0()
                                .when(self.active() == Some(id), |row| row.bg(active_bg))
                                .hover(|style| {
                                    style.bg(if self.active() == Some(id) {
                                        active_bg
                                    } else {
                                        sidebar_hover
                                    })
                                })
                                .gap_1()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .h_full()
                                                .flex()
                                                .items_center()
                                                .child(status_indicator(&tab.info, theme))
                                                .on_mouse_up(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                                                    this.click_sidebar_row(
                                                        SidebarRow::Session(id),
                                                        window,
                                                        cx,
                                                    )
                                                })),
                                        )
                                        .child(
                                            Button::new(("process", id.0))
                                                .small()
                                                .ghost()
                                                .flex_1()
                                                .min_w_0()
                                                .px_0()
                                                .accessibility_label(format!(
                                                    "{full_name} · {detail}"
                                                ))
                                                .tooltip(format!("{full_name} · {detail}"))
                                                .child(
                                                    h_flex()
                                                        .w_full()
                                                        .min_w_0()
                                                        .child(
                                                            div()
                                                                .min_w_0()
                                                                .truncate()
                                                                .when(failed, |label| {
                                                                    label.text_color(theme.danger)
                                                                })
                                                                .when(!running && !failed, |label| label.text_color(sidebar_muted))
                                                                .child(tab.info.name.clone()),
                                                        ),
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.click_sidebar_row(
                                                            SidebarRow::Session(id),
                                                            window,
                                                            cx,
                                                        )
                                                    },
                                                )),
                                        )
                                        .child(
                                            sidebar_actions(
                                                self.sidebar_focus.contains_focused(window, cx)
                                                    && self.sidebar_cursor
                                                        == Some(SidebarRow::Session(id)),
                                            )
                                            .absolute()
                                            .right(px(28.))
                                            .top_0()
                                            .bottom_0()
                                            .px_1()
                                            .gap_1()
                                            .bg(if self.active() == Some(id) {
                                                active_bg
                                            } else {
                                                sidebar_hover
                                            })
                                            .when(running, |actions| actions.child(
                                                session_action(
                                                    ("process-restart", id.0),
                                                    "Restart process",
                                                )
                                                .icon(IconName::RotateCw)
                                                .on_click(cx.listener(move |this, _, _, _| {
                                                    this.core.send(CoreCommand::Restart(id))
                                                })),
                                            ))
                                            .child(
                                                session_action(("process-options", id.0), "Process options")
                                                    .icon(IconName::Ellipsis)
                                                    .dropdown_menu(move |menu, _, _| {
                                                        let edit_workspace = menu_workspace.clone();
                                                        let remove_workspace = menu_workspace.clone();
                                                        menu.item(PopupMenuItem::new("Edit process").on_click(move |_, window, cx| {
                                                            let _ = edit_workspace.update(cx, |this, cx| this.show_edit_process_form(id, window, cx));
                                                        }))
                                                        .item(PopupMenuItem::new("Remove process").on_click(move |_, window, cx| {
                                                            let _ = remove_workspace.update(cx, |this, cx| this.confirm_remove(RemovalTarget::Session(id), window, cx));
                                                        }))
                                                    }),
                                            ),
                                        )
                                        .child(
                                                session_action(
                                                    ("process-toggle", id.0),
                                                    if running {
                                                        "Stop process"
                                                    } else {
                                                        "Start process"
                                                    },
                                                )
                                                .icon(if running {
                                                    IconName::Square
                                                } else {
                                                    IconName::Play
                                                })
                                                .accessibility_label(format!("{} {}", if running { "Stop" } else { "Start" }, tab.info.name))
                                                .on_click(cx.listener(move |this, _, _, _| {
                                                    this.core.send(if running {
                                                        CoreCommand::Stop(id)
                                                    } else {
                                                        CoreCommand::Start(id)
                                                    })
                                                })),
                                            )
                                ),
                        );
                            continue;
                        }
                    }
                    project_panel = project_panel.child(process_rows);
                }
                if terminal_count > 0 {
                    project_panel =
                        project_panel.child(self.terminal_rows(&project.id, window, cx));
                }
                if agent_count > 0 {
                    project_panel = project_panel.child(self.agent_rows(&project.id, window, cx));
                }
                if process_total + terminal_count + agent_count == 0 {
                    let empty_project = project.id.clone();
                    let empty_workspace = cx.weak_entity();
                    project_panel = project_panel.child(
                        Button::new(SharedString::from(format!("empty-project-{empty_project}")))
                            .small()
                            .ghost()
                            .ml_8()
                            .text_color(sidebar_muted)
                            .child("+ Add a process, terminal or agent")
                            .dropdown_menu(move |mut menu, _, cx| {
                                let process_workspace = empty_workspace.clone();
                                let process_project = empty_project.clone();
                                menu = menu.item(PopupMenuItem::new("Add process").on_click(
                                    move |_, window, cx| {
                                        let _ = process_workspace.update(cx, |this, cx| {
                                            this.show_process_form(
                                                process_project.clone(),
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                ));
                                let terminal_workspace = empty_workspace.clone();
                                let terminal_project = empty_project.clone();
                                menu = menu.item(PopupMenuItem::new("New terminal").on_click(
                                    move |_, window, cx| {
                                        let _ = terminal_workspace.update(cx, |this, cx| {
                                            this.active_project = Some(terminal_project.clone());
                                            this.new_terminal(&NewTerminal, window, cx);
                                            this.update_visibility(window, cx);
                                            this.save_layout();
                                        });
                                    },
                                ));
                                let providers = empty_workspace
                                    .upgrade()
                                    .and_then(|view| {
                                        view.read(cx)
                                            .readiness
                                            .as_ref()
                                            .map(crate::setup::Readiness::installed_agents)
                                    })
                                    .unwrap_or_default();
                                for provider in providers {
                                    let agent_workspace = empty_workspace.clone();
                                    let agent_project = empty_project.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(format!("Launch {}", provider.label()))
                                            .on_click(move |_, window, cx| {
                                                let _ = agent_workspace.update(cx, |this, cx| {
                                                    this.show_agent_form(
                                                        agent_project.clone(),
                                                        provider,
                                                        false,
                                                        None,
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                    );
                }
            }
            project_list = project_list.child(project_panel);
        }
        // Library follows the project workspace in the scrollable area.
        project_list = project_list.child(div().h(px(1.)).mx_2().my_3().bg(sidebar_border));
        project_list = project_list
            .child(
                h_flex()
                    .group("sidebar-actions")
                    .px_2()
                    .pt_2()
                    .pb_2()
                    .justify_between()
                    .child(
                        div()
                            .id("library-heading")
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(sidebar_muted)
                            .aria_label("Library")
                            .child("L I B R A R Y"),
                    )
                    .child(
                        session_action("new-skill-sidebar", "New skill")
                            .icon(IconName::Plus)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_skill(window, cx)),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .id("skills-library-row")
                    .aria_expanded(self.skills_expanded)
                    .relative()
                    .min_w_0()
                    .flex_shrink_0()
                    .px_2()
                    .h(px(30.))
                    .rounded_md()
                    .gap_1()
                    .child(self.sidebar_indicator(SidebarRow::Skills, window, cx))
                    .when(skills_overview, |row| row.bg(active_bg))
                    .hover(|style| {
                        style.bg(if skills_overview {
                            active_bg
                        } else {
                            sidebar_hover
                        })
                    })
                    .child(
                        session_action("expand-skills", "Expand or collapse skills")
                            .icon(if self.skills_expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .accessibility_label(if self.skills_expanded {
                                "Collapse skills"
                            } else {
                                "Expand skills"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.skills_expanded = !this.skills_expanded;
                                if this.skills_expanded {
                                    this.refresh_skills(cx);
                                } else if matches!(
                                    this.sidebar_cursor.as_ref(),
                                    Some(SidebarRow::Skill(_) | SidebarRow::SkillSource(_))
                                ) {
                                    this.sidebar_cursor = Some(SidebarRow::Skills);
                                }
                                this.sync_sidebar_cursor();
                                this.save_layout();
                                cx.notify();
                            })),
                    )
                    .child(
                        gpui_kit::component::Icon::default()
                            .data(include_bytes!("../../../assets/icons/bolt.svg"))
                            .size(px(15.))
                            .text_color(sidebar_muted),
                    )
                    .child(
                        Button::new("skills-library")
                            .small()
                            .ghost()
                            .flex_1()
                            .min_w_0()
                            .px_0()
                            .child(div().w_full().text_left().child("Skills"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.click_sidebar_row(SidebarRow::Skills, window, cx)
                            })),
                    ),
            );
        if self.skills_expanded {
            for source in skills::Catalog::current().sources() {
                let source_id = source.id.clone();
                let expanded = self.skill_sources_expanded.contains(&source_id);
                let count = self
                    .skill_source_counts
                    .get(&source_id)
                    .copied()
                    .unwrap_or(0);
                if count == 0 {
                    continue;
                }
                project_list = project_list.child(
                    h_flex()
                        .relative()
                        .min_w_0()
                        .pl_5()
                        .pr_2()
                        .h(px(30.))
                        .gap_2()
                        .child(self.sidebar_indicator(
                            SidebarRow::SkillSource(source_id.clone()),
                            window,
                            cx,
                        ))
                        .child(div().w(px(1.)).h(px(20.)).bg(theme.border))
                        .child(
                            Button::new(SharedString::from(format!("skill-source-{source_id}")))
                                .small()
                                .ghost()
                                .flex_1()
                                .min_w_0()
                                .px_0()
                                .child(div().w_full().text_left().truncate().child(format!(
                                    "{}  {}",
                                    if expanded { "⌄" } else { "›" },
                                    source.agent
                                )))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.click_sidebar_row(
                                        SidebarRow::SkillSource(source_id.clone()),
                                        window,
                                        cx,
                                    )
                                })),
                        ),
                );
                if expanded {
                    for skill in self
                        .skill_items
                        .iter()
                        .filter(|skill| skill.source == source.id)
                    {
                        let id = skill.id.clone();
                        let selected =
                            self.skills_open && self.skill_selected.as_ref() == Some(&id);
                        project_list = project_list.child(
                            h_flex()
                                .relative()
                                .min_w_0()
                                .pl_9()
                                .pr_2()
                                .h(px(30.))
                                .rounded_md()
                                .gap_2()
                                .child(self.sidebar_indicator(
                                    SidebarRow::Skill(id.clone()),
                                    window,
                                    cx,
                                ))
                                .child(div().w(px(1.)).h(px(20.)).bg(theme.border))
                                .when(selected, |row| row.bg(active_bg))
                                .child(
                                    Button::new(SharedString::from(format!("skill-{id}")))
                                        .small()
                                        .ghost()
                                        .flex_1()
                                        .min_w_0()
                                        .px_0()
                                        .child(
                                            div()
                                                .w_full()
                                                .text_left()
                                                .truncate()
                                                .child(skill.name.clone()),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.click_sidebar_row(
                                                SidebarRow::Skill(id.clone()),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    div()
                                        .rounded_md()
                                        .px_1()
                                        .bg(rgb(0x252a33))
                                        .text_size(px(10.))
                                        .font_family("IBM Plex Mono")
                                        .text_color(sidebar_muted)
                                        .child(source.agent.clone()),
                                ),
                        );
                    }
                }
            }
            if self.skill_next_offset.is_some() {
                project_list = project_list.child(
                    Button::new("more-skills-sidebar")
                        .small()
                        .ghost()
                        .ml_6()
                        .px_0()
                        .child(div().w_full().text_left().child("Load more skills"))
                        .on_click(cx.listener(|this, _, _, cx| this.load_more_skills(cx))),
                );
            }
        }
        project_list = project_list.child(
            h_flex()
                .id("notes-library-row")
                .aria_expanded(self.notes_expanded)
                .group("sidebar-actions")
                .relative()
                .min_w_0()
                .flex_shrink_0()
                .px_2()
                .h(px(30.))
                .rounded_md()
                .gap_1()
                .child(self.sidebar_indicator(SidebarRow::Notes, window, cx))
                .when(notes_overview, |row| row.bg(active_bg))
                .hover(|style| {
                    style.bg(if notes_overview {
                        active_bg
                    } else {
                        sidebar_hover
                    })
                })
                .child(
                    session_action("expand-notes", "Expand or collapse notes")
                        .icon(if self.notes_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .accessibility_label(if self.notes_expanded {
                            "Collapse notes"
                        } else {
                            "Expand notes"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.notes_expanded = !this.notes_expanded;
                            if this.notes_expanded {
                                this.refresh_notes(cx);
                            } else if matches!(
                                this.sidebar_cursor.as_ref(),
                                Some(SidebarRow::Note(_))
                            ) {
                                this.sidebar_cursor = Some(SidebarRow::Notes);
                            }
                            this.sync_sidebar_cursor();
                            this.save_layout();
                            cx.notify();
                        })),
                )
                .child(
                    gpui_kit::component::Icon::new(IconName::FileText)
                        .size(px(15.))
                        .text_color(sidebar_muted),
                )
                .child(
                    Button::new("notes-library")
                        .small()
                        .ghost()
                        .flex_1()
                        .min_w_0()
                        .px_0()
                        .child(div().w_full().text_left().child("Notes"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.click_sidebar_row(SidebarRow::Notes, window, cx)
                        })),
                )
                .child(
                    sidebar_actions(
                        self.sidebar_focus.contains_focused(window, cx)
                            && self.sidebar_cursor == Some(SidebarRow::Notes),
                    )
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .px_1()
                    .bg(if notes_overview {
                        active_bg
                    } else {
                        sidebar_hover
                    })
                    .child(
                        session_action("new-note-sidebar", "New note")
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
                    ),
                ),
        );
        if self.notes_expanded {
            for note in &self.notes {
                let id = note.id.clone();
                let selected = self.notes_open && self.note_selected.as_ref() == Some(&id);
                project_list = project_list.child(
                    h_flex()
                        .relative()
                        .min_w_0()
                        .pl_6()
                        .pr_2()
                        .h(px(30.))
                        .rounded_md()
                        .gap_2()
                        .child(self.sidebar_indicator(SidebarRow::Note(id.clone()), window, cx))
                        .when(selected, |row| row.bg(active_bg))
                        .child(div().w(px(1.)).h(px(20.)).bg(theme.border))
                        .child(
                            Button::new(SharedString::from(format!("sidebar-note-{id}")))
                                .small()
                                .ghost()
                                .flex_1()
                                .min_w_0()
                                .px_0()
                                .child(
                                    div()
                                        .w_full()
                                        .text_left()
                                        .truncate()
                                        .child(note.title.clone()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.click_sidebar_row(SidebarRow::Note(id.clone()), window, cx)
                                })),
                        ),
                );
            }
            if self.note_next_offset.is_some() {
                project_list = project_list.child(
                    Button::new("more-notes-sidebar")
                        .small()
                        .ghost()
                        .ml_6()
                        .px_0()
                        .child(div().w_full().text_left().child("Load more notes"))
                        .on_click(cx.listener(|this, _, _, cx| this.load_more_notes(cx))),
                );
            }
        }
        project_list = project_list.child(
            h_flex()
                .id("meetings-library-row")
                .aria_expanded(self.meetings_expanded)
                .relative()
                .min_w_0()
                .flex_shrink_0()
                .px_2()
                .h(px(30.))
                .rounded_md()
                .gap_1()
                .child(self.sidebar_indicator(SidebarRow::Meetings, window, cx))
                .when(self.meetings_open, |row| row.bg(active_bg))
                .hover(|style| {
                    style.bg(if self.meetings_open {
                        active_bg
                    } else {
                        sidebar_hover
                    })
                })
                .child(
                    session_action("expand-meetings", "Expand or collapse meetings")
                        .icon(if self.meetings_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .accessibility_label(if self.meetings_expanded {
                            "Collapse meetings"
                        } else {
                            "Expand meetings"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.meetings_expanded = !this.meetings_expanded;
                            if this.meetings_expanded {
                                this.core.send(CoreCommand::ListMeetings);
                            } else if matches!(
                                this.sidebar_cursor.as_ref(),
                                Some(SidebarRow::Meeting(_))
                            ) {
                                this.sidebar_cursor = Some(SidebarRow::Meetings);
                            }
                            this.sync_sidebar_cursor();
                            this.save_layout();
                            cx.notify();
                        })),
                )
                .child(
                    gpui_kit::component::Icon::new(IconName::Mic)
                        .size(px(15.))
                        .text_color(sidebar_muted),
                )
                .child(
                    Button::new("meetings-library")
                        .small()
                        .ghost()
                        .flex_1()
                        .min_w_0()
                        .px_0()
                        .child(div().w_full().text_left().child("Meetings"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.click_sidebar_row(SidebarRow::Meetings, window, cx)
                        })),
                ),
        );
        if self.meetings_expanded {
            let mut last_day = None;
            for meeting in &self.meetings {
                let started = Local.timestamp_opt(meeting.started_at, 0).single();
                let day = started.as_ref().map(|time| time.date_naive());
                if day != last_day {
                    let day_label = started.as_ref().map_or_else(
                        || "Unknown date".to_string(),
                        |time| {
                            if time.date_naive() == Local::now().date_naive() {
                                "Today".to_string()
                            } else {
                                time.format("%a, %b %-d").to_string()
                            }
                        },
                    );
                    project_list = project_list.child(
                        div()
                            .pl_9()
                            .pt_2()
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(sidebar_muted)
                            .child(day_label),
                    );
                    last_day = day;
                }
                let id = meeting.id.clone();
                let selected = self.meetings_open && self.meeting_selected.as_ref() == Some(&id);
                project_list = project_list.child(
                    h_flex()
                        .relative()
                        .min_w_0()
                        .pl_6()
                        .pr_2()
                        .h(px(30.))
                        .rounded_md()
                        .gap_2()
                        .child(self.sidebar_indicator(SidebarRow::Meeting(id.clone()), window, cx))
                        .child(div().w(px(1.)).h(px(20.)).bg(theme.border))
                        .when(selected, |row| row.bg(active_bg))
                        .child(
                            Button::new(SharedString::from(format!("sidebar-meeting-{id}")))
                                .small()
                                .ghost()
                                .flex_1()
                                .min_w_0()
                                .px_0()
                                .child(
                                    div()
                                        .w_full()
                                        .text_left()
                                        .truncate()
                                        .child(meeting.title.clone()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.click_sidebar_row(
                                        SidebarRow::Meeting(id.clone()),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("IBM Plex Mono")
                                .text_color(sidebar_muted)
                                .child(started.map_or_else(
                                    || "—".to_string(),
                                    |time| time.format("%-I:%M %p").to_string(),
                                )),
                        ),
                );
            }
            if self.meetings.is_empty() {
                project_list = project_list.child(
                    div()
                        .pl_6()
                        .py_2()
                        .text_xs()
                        .text_color(muted)
                        .child("No meetings yet"),
                );
            }
        }
        sidebar = sidebar.child(project_list);
        let running_count = self
            .tabs
            .values()
            .filter(|tab| {
                matches!(
                    tab.info.status,
                    SessionStatus::Running | SessionStatus::Starting
                )
            })
            .count();
        let memory_bytes: u64 = self
            .tabs
            .values()
            .filter_map(|tab| {
                self.memory
                    .get(&tab.info.id)
                    .filter(|(launch, _)| {
                        *launch == tab.info.launch
                            && matches!(
                                tab.info.status,
                                SessionStatus::Running | SessionStatus::Stopping
                            )
                    })
                    .and_then(|(_, bytes)| *bytes)
            })
            .sum();
        sidebar = sidebar.child(
            h_flex()
                .h(px(42.))
                .px_3()
                .gap_2()
                .border_t_1()
                .border_color(sidebar_border)
                .child(div().size(px(7.)).rounded_full().bg(rgb(0x3fcf8e)))
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(sidebar_muted)
                        .child(format!(
                            "{running_count} running · {:.0} MB",
                            memory_bytes as f64 / 1_000_000.
                        )),
                )
                .child(
                    div()
                        .relative()
                        .child(
                            session_action("sidebar-settings", "Settings")
                                .icon(IconName::Settings)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.setup_open = true;
                                    this.focus.focus(window, cx);
                                    this.check_setup(cx);
                                })),
                        )
                        .when(self.update_available.is_some(), |button| {
                            button.child(
                                div()
                                    .absolute()
                                    .top(px(1.))
                                    .right(px(1.))
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(rgb(0x3fcf8e))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _, window, cx| {
                                            this.setup_open = true;
                                            this.focus.focus(window, cx);
                                            this.check_setup(cx);
                                        }),
                                    ),
                            )
                        }),
                ),
        );
        self.sidebar_scroll_pending = false;
        let visible = self.visible_ids();
        let project_name = self
            .projects
            .iter()
            .find(|project| Some(&project.id) == self.active_project.as_ref())
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "Workspace".into());
        let project_name = if self.skills_open || self.notes_open || self.meetings_open {
            "Library".to_string()
        } else {
            project_name
        };
        let mut content = v_flex().flex_1().min_w_0().h_full().child(
            h_flex()
                .px_4()
                .py_2()
                .gap_3()
                .border_b_1()
                .border_color(border)
                .child(
                    Button::new("toggle-workspace-sidebar")
                        .small()
                        .ghost()
                        .icon(if self.sidebar_collapsed {
                            IconName::PanelLeftOpen
                        } else {
                            IconName::PanelLeftClose
                        })
                        .tooltip(if self.sidebar_collapsed {
                            "Show sidebar · ⌘B"
                        } else {
                            "Hide sidebar · ⌘B"
                        })
                        .accessibility_label(if self.sidebar_collapsed {
                            "Show sidebar"
                        } else {
                            "Hide sidebar"
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_sidebar(&ToggleSidebar, window, cx)
                        })),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .truncate()
                                .child(project_name),
                        )
                        .child(
                            div().text_sm().font_semibold().truncate().child(
                                self.active()
                                    .and_then(|id| self.tabs.get(&id))
                                    .map(|tab| tab.info.name.clone())
                                    .unwrap_or_else(|| {
                                        if self.skills_open {
                                            "Skills"
                                        } else if self.notes_open {
                                            "Notes"
                                        } else if self.meetings_open {
                                            "Meetings"
                                        } else {
                                            "Overview"
                                        }
                                        .into()
                                    }),
                            ),
                        ),
                )
                .child(if let Some(meeting) = self.active_recording() {
                    let elapsed = (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64
                        - meeting.started_at)
                        .max(0) as u64;
                    let label = match meeting.status {
                        meetings::Status::Starting => "Starting…".to_string(),
                        meetings::Status::Stopping => "Saving…".to_string(),
                        _ => format!("Stop · {:02}:{:02}", elapsed / 60, elapsed % 60),
                    };
                    Button::new("stop-meeting-toolbar")
                        .small()
                        .primary()
                        .label(label)
                        .disabled(meeting.status != meetings::Status::Recording)
                        .on_click(cx.listener(|this, _, _, cx| this.stop_meeting(cx)))
                } else {
                    Button::new("record-meeting-toolbar")
                        .small()
                        .primary()
                        .label("Record")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.record_meeting(window, cx)),
                        )
                })
                .when(
                    self.active_project.is_some()
                        && !self.skills_open
                        && !self.notes_open
                        && !self.meetings_open,
                    |header| {
                        header.child(
                            Button::new("toolbar-new-terminal")
                                .small()
                                .icon(IconName::Plus)
                                .label("Terminal")
                                .tooltip("New terminal · ⌘T")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.new_terminal(&NewTerminal, window, cx)
                                })),
                        )
                    },
                ),
        );
        if self.skills_open {
            content = content.child(self.skills_content(cx));
        } else if self.notes_open {
            content = content.child(self.notes_content(cx));
        } else if self.meetings_open {
            content = content.child(self.meetings_content(cx));
        } else if let Some(side) = self
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
                        .border_b_1()
                        .border_color(border)
                        .child(div().flex_1().min_w_0().truncate().child(chrome))
                        .child(
                            session_action("close-side", "Close side conversation")
                                .icon(IconName::Close)
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
            let sidebar_size = if self.sidebar_collapsed {
                0.
            } else {
                self.sidebar_state
                    .read(cx)
                    .sizes()
                    .first()
                    .copied()
                    .map(f32::from)
                    .unwrap_or(self.sidebar_width)
            };
            if f32::from(window.viewport_size().width) - sidebar_size < 600. {
                // Stack conversations when side-by-side terminals would be too
                // narrow; keep the desktop split state for the next wide render.
                content = content.child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .child(self.tabs[&parent].view.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .border_t_1()
                                .border_color(border)
                                .child(side_pane),
                        ),
                );
            } else {
                content = content.child(
                    div().flex_1().min_h_0().child(
                        h_resizable(("agent-split", parent.0))
                            .with_state(&split_state)
                            .child(resizable_panel().child(self.tabs[&parent].view.clone()))
                            .child(resizable_panel().child(side_pane)),
                    ),
                );
            }
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
                    .p_6()
                    .gap_4()
                    .child(div().text_lg().font_semibold().child(if self.active_project.is_some() {
                        "Open a terminal to get started"
                    } else { "Your development workspace, together." }))
                    .child(div().max_w(px(360.)).text_sm().text_color(muted).text_center()
                        .child(if self.active_project.is_some() {
                            "Run a shell, start a configured process, or launch an agent from the sidebar."
                        } else { "Bring your terminals, project processes, and coding agents into one focused desktop workspace." }))
                    .child(Button::new("empty-primary-action").primary()
                        .label(if self.active_project.is_some() { "New terminal" } else { "Add project" })
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.active_project.is_some() { this.new_terminal(&NewTerminal, window, cx); }
                            else { this.add_project(&AddProject, window, cx); }
                        })))
                    .when(self.projects.is_empty(), |empty| empty.child(self.setup_summary(cx)))
                    .child(div().text_xs().text_color(muted).child("⌘P  Search all commands")),
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
                if event.keystroke.key == "escape" && this.note_modal_open {
                    this.cancel_new_note_modal(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.setup_open {
                    this.dismiss_setup(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.process_project.is_some() {
                    this.cancel_process(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && this.meeting_record_choice {
                    this.meeting_record_choice = false;
                    this.restore_focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                } else if event.keystroke.key == "escape" && this.agent_launch.is_some() {
                    this.cancel_agent_form(window, cx);
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
            .on_action(cx.listener(Self::show_new_note_modal))
            .on_action(cx.listener(Self::close_terminal))
            .on_action(cx.listener(Self::rename_terminal))
            .on_action(cx.listener(Self::ask_agent_question))
            .on_action(cx.listener(Self::add_project))
            .on_action(cx.listener(Self::focus_sidebar))
            .on_action(cx.listener(Self::toggle_sidebar))
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
            body = body.child(
                div()
                    .px_4()
                    .py_3()
                    .text_sm()
                    .bg(theme.danger.opacity(0.1))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        // Leave a usable primary pane when a wide saved sidebar meets a
        // smaller window. The preference is still available on larger windows.
        let sidebar_max = (f32::from(window.viewport_size().width) - 320.).clamp(220., 520.);
        let sidebar_owner = cx.weak_entity();
        body = body.child(
            if self.perf_layout == "no-sidebar" || self.sidebar_collapsed {
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
                            .size(px(self.sidebar_width.min(sidebar_max)))
                            .flex_none()
                            .size_range(px(220.)..px(sidebar_max))
                            .child(sidebar),
                    )
                    .child(
                        resizable_panel()
                            .size_range(px(320.)..Pixels::MAX)
                            .child(content),
                    )
                    .into_any_element()
            },
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
                    .occlude()
                    .bg(hsla(0., 0., 0., 0.3))
                    .flex()
                    .justify_center()
                    .p_4()
                    .pt(px(64.))
                    .child(div().w(px(560.)).max_w_full().h_auto().child(palette)),
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
                    .border_t_1()
                    .border_color(border)
                    .bg(theme.sidebar)
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
            body = body.child(div().absolute().inset_0().occlude().bg(hsla(0., 0., 0., 0.4))
                .flex().justify_center().p_4().pt(px(64.)).child(
                    dialog_panel(720., theme).h(px(560.)).max_h(px((f32::from(window.viewport_size().height) - 96.).max(120.)))
                        .child(div().font_semibold().child("Review customization"))
                        .child(div().text_sm().text_color(muted).child("Check commands and autostart before applying. Appearance updates immediately; restart processes for command changes. Autostart applies on project open. Undo restores configuration, but cannot undo executed commands."))
                        .child(v_flex().id("customization-proposal").flex_1().min_h_0().overflow_y_scroll().p_3().rounded_md().bg(theme.muted).text_sm().font_family(theme.mono_font_family.clone())
                            .children(preview.changes.lines().chain(std::iter::once("— Complete proposed configuration —")).chain(proposal.lines()).enumerate().map(|(index, line)| {
                                div().id(SharedString::from(format!("proposal-line-{index}"))).when(line.starts_with("+ "), |row| row.text_color(theme.success)).when(line.starts_with("− "), |row| row.text_color(theme.danger)).child(line.to_string())
                            })))
                        .child(h_flex().gap_3()
                            .child(Button::new("apply-customization").primary().label("Apply changes")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.core.send(CoreCommand::ApplyCustomization(reviewed.clone()));
                                    this.customization_preview = None;
                                    this.restore_focus(window, cx);
                                    cx.notify();
                                })))
                            .child(Button::new("cancel-customization").ghost().label("Cancel")
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
                    .occlude()
                    .bg(hsla(0., 0., 0., 0.3))
                    .flex()
                    .justify_center()
                    .items_start()
                    .p_4().pt(px(64.))
                    .child(
                        dialog_panel(560., theme)
                            .child(div().font_semibold().child(if self.customization_provider.is_some() {
                                "Customize Tervy"
                            } else if self.question_parent.is_some() {
                                "Ask a read-only question"
                            } else {
                                "Rename terminal"
                            }))
                            .when(self.customization_provider.is_some(), |panel| panel.child(div().text_sm().text_color(muted).child("Try: use warm-dark, set text to 16, and narrow the sidebar. The agent prepares a proposal; choose Review Customization in Cmd-P when ready.")))
                            .child(v_flex().gap_2()
                                .when(self.perf_layout == "modal-focus", |group| group.on_children_prepainted(modal_focus_smoke::measure("Rename", 1)))
                                .child(div().text_sm().child(if self.customization_provider.is_some() { "Describe your changes" }
                                    else if self.question_parent.is_some() { "Question" } else { "Terminal name" }))
                                .child(modal_input(&self.rename_input, if self.customization_provider.is_some() { "Describe your changes" } else if self.question_parent.is_some() { "Question" } else { "Terminal name" })))
                            .child(
                                h_flex()
                                    .gap_4()
                                    .child(
                                        Button::new("rename-save").primary().label(if self.customization_provider.is_some() {
                                                "Start conversation"
                                            } else if self.question_parent.is_some() {
                                                "Ask question"
                                            } else {
                                                "Save name"
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.save_rename(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("rename-cancel").ghost().label("Cancel")
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
        if self.setup_open {
            body = body.child(self.setup_dialog(window, cx));
        }
        if self.process_project.is_some() {
            body = body.child(self.process_form(cx));
        }
        if self.meeting_record_choice {
            body = body.child(self.meeting_record_dialog(cx));
        }
        if self.agent_launch.is_some() {
            body = body.child(self.agent_form(cx));
        }
        if self.note_modal_open {
            body = body.child(self.new_note_modal(window, cx));
        }
        if let Some(remaining) = self.shutdown {
            body = body.child(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
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
        assert!(fuzzy_match("Switch Project: Tervy", "s p ty"));
        assert!(fuzzy_match("New Terminal", "NWT"));
        assert!(!fuzzy_match("New Terminal", "ntz"));
    }
}
