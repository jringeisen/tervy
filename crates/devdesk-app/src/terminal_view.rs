//! Presentation and direct PTY input only. The supervisor owns each session;
//! dropping or hiding this view never stops its process.

use std::ops::Range;

use devdesk_core::agent;
use devdesk_core::config::Settings;
use devdesk_core::supervisor::{Command, LaunchId, SessionId, SessionStatus, SupervisorHandle};
use devdesk_term::keys::{self, Key, KeyEvent, KeyInput, MouseAction};
use devdesk_term::snapshot::{Palette, RenderSnapshot};
use devdesk_term::{
    Dimensions, Event, Scroll, TermMode, TerminalEvents, TerminalHandle, TerminalSignal,
};
use gpui_kit::component::{Theme, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::terminal_element::{CellMetrics, TerminalElement, color as terminal_color};

gpui_kit::actions!(devdesk_terminal, [SendTab, SendBacktab]);

/// Action bindings resolve before raw keys. Handle Tab at the focused terminal
/// itself so root focus traversal cannot consume it, while inputs keep traversal.
pub fn key_bindings() -> [KeyBinding; 2] {
    [
        KeyBinding::new("tab", SendTab, Some("Terminal")),
        KeyBinding::new("shift-tab", SendBacktab, Some("Terminal")),
    ]
}

pub struct RenderedTerminal {
    pub handle: TerminalHandle,
}

pub struct TerminalView {
    id: SessionId,
    launch: LaunchId,
    provider: Option<agent::Provider>,
    core: SupervisorHandle,
    visible: bool,
    pub font_size: f32,
    pub focus: FocusHandle,
    pub session: Option<RenderedTerminal>,
    pub size: Dimensions,
    pub palette: Palette,
    pub bounds: Bounds<Pixels>,
    pub metrics: CellMetrics,
    pub snapshot: Option<RenderSnapshot>,
    pub preedit: String,
    pub font_family: SharedString,
    pub probe: Option<crate::terminal_perf::TerminalProbe>,
    preedit_selection: Range<usize>,
    warning: Option<String>,
    status: String,
    exited: bool,
    focused: bool,
    active: bool,
    bell: bool,
    clipboard_notice: bool,
    selecting: bool,
    scroll_remainder: f32,
    option_as_meta: bool,
    subscriptions: Vec<Subscription>,
    tasks: Vec<Task<()>>,
}

impl TerminalView {
    pub fn configure(&mut self, settings: &Settings, cx: &mut Context<Self>) {
        self.font_size = settings.font_size;
        self.font_family = std::env::var("DEVDESK_FONT_FAMILY")
            .unwrap_or_else(|_| settings.font_family.clone())
            .into();
        self.option_as_meta = settings.option_as_meta
            || std::env::var("DEVDESK_OPTION_AS_META").as_deref() == Ok("1");
        self.palette = crate::theme::terminal_palette(cx);
        if let Some(session) = &self.session {
            session.handle.invalidate();
        }
        cx.notify();
    }

    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        id: SessionId,
        core: SupervisorHandle,
        settings: &Settings,
    ) -> Self {
        let _phase = crate::perf::phase("terminal view init");
        let focus = cx.focus_handle();
        let mut view = Self {
            id,
            launch: LaunchId(0),
            provider: None,
            core,
            visible: false,
            font_size: settings.font_size,
            focus,
            session: None,
            size: Dimensions::default(),
            palette: crate::theme::terminal_palette(cx),
            bounds: Bounds::default(),
            metrics: CellMetrics::default(),
            snapshot: None,
            preedit: String::new(),
            probe: crate::terminal_perf::TerminalProbe::from_env(),
            font_family: std::env::var("DEVDESK_FONT_FAMILY")
                .unwrap_or_else(|_| settings.font_family.clone())
                .into(),
            preedit_selection: 0..0,
            warning: None,
            status: "Starting login shell…".into(),
            exited: false,
            focused: true,
            active: window.is_window_active(),
            bell: false,
            clipboard_notice: false,
            selecting: false,
            scroll_remainder: 0.,
            option_as_meta: settings.option_as_meta
                || std::env::var("DEVDESK_OPTION_AS_META").as_deref() == Ok("1"),
            subscriptions: Vec::new(),
            tasks: Vec::new(),
        };
        view.subscriptions
            .push(cx.observe_global::<Theme>(|view, cx| {
                let palette = crate::theme::terminal_palette(cx);
                if view.palette != palette {
                    view.palette = palette;
                    // No process restart or terminal reset: snapshots resolve
                    // existing cells through the new palette on the next paint.
                    if let Some(session) = &view.session {
                        session.handle.invalidate();
                    }
                    cx.notify();
                }
            }));
        view.subscriptions
            .push(cx.on_focus(&view.focus, window, |view, _, cx| {
                view.focused = true;
                view.send_focus(view.active && view.visible);
                cx.notify();
            }));
        view.subscriptions
            .push(cx.on_blur(&view.focus, window, |view, _, cx| {
                view.focused = false;
                view.send_focus(false);
                cx.notify();
            }));
        view.subscriptions
            .push(cx.observe_window_activation(window, |view, window, cx| {
                view.active = window.is_window_active();
                view.send_focus(view.active && view.focused && view.visible);
                cx.notify();
            }));
        view
    }

    pub fn set_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.send_focus(visible && self.active);
        if visible {
            self.focus.focus(window, cx);
            if let Some(session) = &self.session {
                session.handle.invalidate();
            }
            cx.notify();
        }
    }

    pub fn attach(
        &mut self,
        handle: TerminalHandle,
        events: TerminalEvents,
        warning: Option<String>,
        spawn_started: Option<std::time::Instant>,
        cx: &mut Context<Self>,
    ) {
        self.tasks.clear();
        self.snapshot = None;
        self.preedit.clear();
        self.exited = false;
        self.warning = warning;
        self.status = "Login shell".into();
        if let Some(probe) = self.probe.as_mut()
            && let Some(started) = spawn_started
        {
            probe.record_spawn_started(started);
        }
        handle.resize(self.size);
        self.session = Some(RenderedTerminal { handle });
        self.listen(events, cx);
        if self.visible {
            cx.notify();
        }
    }

    pub fn set_status(&mut self, status: &SessionStatus, cx: &mut Context<Self>) {
        self.exited = matches!(
            status,
            SessionStatus::Stopped | SessionStatus::Exited { .. } | SessionStatus::Failed(_)
        );
        self.status = match status {
            SessionStatus::Stopped => "Stopped — Return to start".into(),
            SessionStatus::Starting => "Starting…".into(),
            SessionStatus::Running => "Running".into(),
            SessionStatus::Stopping => "Stopping…".into(),
            SessionStatus::Exited { code } => format!(
                "Exited with {} — Return to restart",
                code.map_or_else(|| "signal".into(), |code| code.to_string())
            ),
            SessionStatus::Failed(error) => {
                format!("{error} — Return to retry")
            }
        };
        if self.visible {
            cx.notify();
        }
    }

    pub fn set_agent(&mut self, info: Option<&agent::Info>, launch: LaunchId) {
        self.launch = launch;
        self.provider = info.map(|info| info.record.provider);
        if info.is_some_and(|info| {
            info.record.provider == agent::Provider::Codex
                && info.status != agent::Status::Attention
        }) {
            self.bell = false;
        }
    }
    pub fn set_agent_stub(&mut self, cx: &mut Context<Self>) {
        self.exited = true;
        self.status = format!(
            "{} conversation · Return to resume",
            self.provider.map_or("Agent", agent::Provider::label)
        );
        if self.visible {
            cx.notify();
        }
    }

    fn listen(&mut self, events: TerminalEvents, cx: &mut Context<Self>) {
        let launch = self.launch;
        self.tasks.push(cx.spawn(async move |view, cx| {
            while let Ok(query) = events.queries.recv().await {
                if view
                    .update(cx, |view, _| {
                        if view.launch != launch {
                            return;
                        }
                        if let Some(session) = &view.session {
                            session
                                .handle
                                .answer_query(&query, &view.palette, view.size);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        self.tasks.push(cx.spawn(async move |view, cx| {
            while let Ok(signal) = events.signals.recv().await {
                let done = matches!(signal, TerminalSignal::Exit);
                if view
                    .update(cx, |view, cx| {
                        if view.launch != launch {
                            return;
                        }
                        if done
                            && view.provider == Some(agent::Provider::Codex)
                            && let Some(session) = &view.session
                            && let Some(session_id) =
                                exit_summary_id(&session.handle.snapshot(&view.palette))
                        {
                            view.core.send(Command::AgentExitSummary {
                                id: view.id,
                                launch,
                                session_id,
                            });
                        }
                        // Core owns lifecycle state, including session cleanup after
                        // leader exit. This stream only coalesces output redraws.
                        if view.visible {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
                if done {
                    break;
                }
            }
        }));
        self.tasks.push(cx.spawn(async move |view, cx| {
            while let Ok(event) = events.events.recv().await {
                if view
                    .update(cx, |view, cx| {
                        if view.launch != launch {
                            return;
                        }
                        if let Some(session) = &view.session
                            && session
                                .handle
                                .answer_query(&event, &view.palette, view.size)
                        {
                            return;
                        }
                        match event {
                            Event::Title(title) => {
                                if view.provider == Some(agent::Provider::Codex) {
                                    view.core.send(Command::AgentTitle {
                                        id: view.id,
                                        launch,
                                        title,
                                    });
                                    return;
                                }
                                view.status = title
                                    .chars()
                                    .filter(|c| !c.is_control())
                                    .take(200)
                                    .collect();
                                if view.visible {
                                    cx.notify();
                                }
                            }
                            Event::ResetTitle => {
                                view.status = "Login shell".into();
                                if view.visible {
                                    cx.notify();
                                }
                            }
                            Event::Bell => {
                                if !view.bell && view.provider == Some(agent::Provider::Codex) {
                                    view.core.send(Command::AgentBell {
                                        id: view.id,
                                        launch,
                                    });
                                }
                                view.bell = true;
                                if view.visible {
                                    cx.notify();
                                }
                            }
                            Event::ClipboardStore(_, text)
                                if view.visible && view.focused && view.active =>
                            {
                                // Clipboard is capped in EventProxy, and read requests
                                // are denied there. A visible notice accompanies writes.
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                                view.clipboard_notice = true;
                                if view.visible {
                                    cx.notify();
                                }
                            }
                            _ => {}
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn send_focus(&self, focused: bool) {
        if let Some(session) = &self.session {
            session
                .handle
                .write(keys::focus(focused, session.handle.mode()));
        }
    }

    fn tab(&mut self, _: &SendTab, window: &mut Window, cx: &mut Context<Self>) {
        self.send_tab(false, window, cx);
    }
    fn backtab(&mut self, _: &SendBacktab, window: &mut Window, cx: &mut Context<Self>) {
        self.send_tab(true, window, cx);
    }
    fn send_tab(&mut self, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = &self.session
            && self.preedit.is_empty()
        {
            let mode = session.handle.mode();
            let input = KeyInput {
                key: Key::Tab,
                modifiers: keys::Modifiers {
                    shift,
                    ..Default::default()
                },
                event: KeyEvent::Press,
                text: Some("\t".into()),
                shifted: None,
                base_layout: None,
            };
            let bytes = keys::encode(&input, mode, self.option_as_meta);
            if std::env::var_os("DEVDESK_TRACE_TAB").is_some() {
                eprintln!("devdesk-tab action shift={shift} mode={mode:?} bytes={bytes:?}");
            }
            self.write_input(bytes);
            self.bell = false;
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        if stroke.modifiers.shift
            && !stroke.modifiers.platform
            && !stroke.modifiers.control
            && !stroke.modifiers.alt
            && matches!(stroke.key.as_str(), "pageup" | "pagedown")
        {
            if let Some(session) = &self.session {
                session
                    .handle
                    .term
                    .lock()
                    .scroll_display(if stroke.key == "pageup" {
                        Scroll::PageUp
                    } else {
                        Scroll::PageDown
                    });
                session.handle.invalidate();
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if stroke.modifiers.platform {
            match stroke.key.as_str() {
                "a" => {
                    if let Some(session) = &self.session {
                        use devdesk_term::GridDimensions as _;
                        let mut term = session.handle.term.lock();
                        let mut selection = devdesk_term::Selection::new(
                            devdesk_term::SelectionType::Simple,
                            devdesk_term::Point::new(term.topmost_line(), devdesk_term::Column(0)),
                            devdesk_term::Side::Left,
                        );
                        selection.update(
                            devdesk_term::Point::new(term.bottommost_line(), term.last_column()),
                            devdesk_term::Side::Right,
                        );
                        selection.include_all();
                        term.selection = Some(selection);
                        drop(term);
                        session.handle.invalidate();
                    }
                }
                "home" | "end" => {
                    if let Some(session) = &self.session {
                        session
                            .handle
                            .term
                            .lock()
                            .scroll_display(if stroke.key == "home" {
                                Scroll::Top
                            } else {
                                Scroll::Bottom
                            });
                        session.handle.invalidate();
                    }
                }
                "c" => {
                    if let Some(session) = &self.session
                        && let Some(text) = session.handle.term.lock().selection_to_string()
                    {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                "v" => {
                    if let Some(item) = cx.read_from_clipboard()
                        && let Some(text) = item.text()
                    {
                        self.paste_text(&text);
                    }
                }
                "k" => {
                    if let Some(session) = &self.session {
                        {
                            let mut term = session.handle.term.lock();
                            term.grid_mut().clear_history();
                            term.grid_mut().truncate();
                        }
                        session.handle.invalidate();
                    }
                }
                _ => return,
            }
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if self.exited && stroke.key == "enter" {
            self.core.send(Command::Restart(self.id));
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        let Some(session) = &self.session else { return };
        let mode = session.handle.mode();
        let Some(input) = convert_key(
            stroke,
            if event.is_held {
                KeyEvent::Repeat
            } else {
                KeyEvent::Press
            },
        ) else {
            return;
        };
        let native_text = matches!(input.key, Key::Character(_))
            && !input.modifiers.control
            && !(input.modifiers.alt && self.option_as_meta)
            && !mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
        // Let AppKit/GPUI interpret ordinary text, Option accents and IME.
        // Control/function/kitty keys are consumed here to avoid double sends.
        if native_text || !self.preedit.is_empty() {
            return;
        }
        self.write_input(keys::encode(&input, mode, self.option_as_meta));
        if input.key == Key::Enter {
            self.input_notice(true);
        }
        self.bell = false;
        window.prevent_default();
        cx.stop_propagation();
    }

    fn key_up(&mut self, event: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.platform || !self.preedit.is_empty() {
            return;
        }
        if let Some(session) = &self.session
            && let Some(input) = convert_key(&event.keystroke, KeyEvent::Release)
        {
            let bytes = keys::encode(&input, session.handle.mode(), self.option_as_meta);
            if !bytes.is_empty() {
                session.handle.write(bytes);
                window.prevent_default();
                cx.stop_propagation();
            }
        }
    }

    fn input_notice(&self, enter: bool) {
        if self.provider == Some(agent::Provider::Codex) {
            self.core.send(Command::AgentInput {
                id: self.id,
                launch: self.launch,
                enter,
            });
        }
    }
    fn write_input(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        self.clipboard_notice = false;
        self.bell = false;
        self.input_notice(false);
        if let Some(session) = &self.session {
            {
                let mut term = session.handle.term.lock();
                term.selection = None;
                term.scroll_display(Scroll::Bottom);
            }
            session.handle.write(bytes);
            if let Some(probe) = self.probe.as_mut() {
                probe.record_write();
            }
            session.handle.invalidate();
        }
    }

    fn paste_text(&mut self, text: &str) {
        if let Some(session) = &self.session {
            self.write_input(keys::paste(text, session.handle.mode()));
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        self.paste_text(text);
    }

    fn cell(&self, position: Point<Pixels>) -> (usize, usize) {
        let col = ((position.x - self.bounds.origin.x) / self.metrics.width)
            .floor()
            .max(0.) as usize;
        let row = ((position.y - self.bounds.origin.y) / self.metrics.height)
            .floor()
            .max(0.) as usize;
        (
            col.min(self.size.columns as usize - 1),
            row.min(self.size.rows as usize - 1),
        )
    }

    fn mouse_report(
        &self,
        action: MouseAction,
        position: Point<Pixels>,
        modifiers: Modifiers,
    ) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        if modifiers.shift {
            return false;
        }
        let (col, row) = self.cell(position);
        let mode = session.handle.mode();
        if !mode.intersects(TermMode::MOUSE_MODE) {
            return false;
        }
        session.handle.write(keys::mouse(
            action,
            col,
            row,
            key_modifiers(modifiers),
            mode,
        ));
        true
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.bell = false;
        if event.modifiers.platform && event.button == MouseButton::Left {
            let (col, row) = self.cell(event.position);
            if let Some(snapshot) = &self.snapshot
                && let Some(link) = snapshot.hyperlinks.iter().find(|link| {
                    link.row == row && col >= link.column && col < link.column + link.cells
                })
                && (link.uri.starts_with("https://")
                    || link.uri.starts_with("http://")
                    || link.uri.starts_with("file://"))
            {
                cx.open_url(&link.uri);
            }
            return;
        }
        if self.mouse_report(
            MouseAction::Press(mouse_button(event.button)),
            event.position,
            event.modifiers,
        ) {
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let (col, row) = self.cell(event.position);
        if let Some(session) = &self.session {
            let mut term = session.handle.term.lock();
            let point = devdesk_term::Point::new(
                devdesk_term::Line(
                    row as i32 - self.snapshot.as_ref().map_or(0, |s| s.display_offset) as i32,
                ),
                devdesk_term::Column(col),
            );
            let kind = match event.click_count {
                2 => devdesk_term::SelectionType::Semantic,
                3.. => devdesk_term::SelectionType::Lines,
                _ => devdesk_term::SelectionType::Simple,
            };
            let mut selection = devdesk_term::Selection::new(kind, point, devdesk_term::Side::Left);
            if event.click_count >= 2 {
                selection.include_all();
            }
            term.selection = Some(selection);
            drop(term);
            session.handle.invalidate();
            self.selecting = true;
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, _: &mut Context<Self>) {
        if self.mouse_report(
            MouseAction::Motion(event.pressed_button.map(mouse_button)),
            event.position,
            event.modifiers,
        ) {
            return;
        }
        if self.selecting && event.dragging() {
            let (col, row) = self.cell(event.position);
            if let Some(session) = &self.session {
                let mut term = session.handle.term.lock();
                let offset = term.grid().display_offset();
                if let Some(selection) = term.selection.as_mut() {
                    selection.update(
                        devdesk_term::Point::new(
                            devdesk_term::Line(row as i32 - offset as i32),
                            devdesk_term::Column(col),
                        ),
                        devdesk_term::Side::Right,
                    );
                }
                drop(term);
                session.handle.invalidate();
            }
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.mouse_report(
            MouseAction::Release(mouse_button(event.button)),
            event.position,
            event.modifiers,
        );
        self.selecting = false;
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = match event.delta {
            ScrollDelta::Pixels(delta) => delta.y / self.metrics.height,
            ScrollDelta::Lines(delta) => delta.y,
        };
        self.scroll_remainder += delta;
        let lines = self.scroll_remainder.trunc() as i32;
        self.scroll_remainder -= lines as f32;
        if lines == 0 {
            return;
        }
        if self.mouse_report(
            if lines > 0 {
                MouseAction::WheelUp
            } else {
                MouseAction::WheelDown
            },
            event.position,
            event.modifiers,
        ) {
            for _ in 1..lines.unsigned_abs().min(100) {
                self.mouse_report(
                    if lines > 0 {
                        MouseAction::WheelUp
                    } else {
                        MouseAction::WheelDown
                    },
                    event.position,
                    event.modifiers,
                );
            }
        } else if let Some(session) = &self.session {
            if session
                .handle
                .mode()
                .contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
                && !event.modifiers.shift
            {
                let input = KeyInput {
                    key: if lines > 0 { Key::Up } else { Key::Down },
                    modifiers: keys::Modifiers::default(),
                    event: KeyEvent::Press,
                    text: None,
                    shifted: None,
                    base_layout: None,
                };
                let bytes = keys::encode(&input, session.handle.mode(), false);
                for _ in 0..lines.unsigned_abs().min(100) {
                    session.handle.write(bytes.clone());
                }
            } else {
                session
                    .handle
                    .term
                    .lock()
                    .scroll_display(Scroll::Delta(lines));
                session.handle.invalidate();
            }
        }
        cx.stop_propagation();
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.clone();
        let warning = self.warning.as_deref().unwrap_or("");
        v_flex()
            .size_full()
            .bg(terminal_color(self.palette.background))
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .id("terminal")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    // The element receives the inset content bounds, keeping
                    // PTY sizing, selection and IME coordinates aligned.
                    .p(px(12.))
                    .overflow_hidden()
                    .track_focus(&self.focus)
                    .key_context("Terminal")
                    .on_action(cx.listener(Self::tab))
                    .on_action(cx.listener(Self::backtab))
                    .on_key_down(cx.listener(Self::key))
                    .on_key_up(cx.listener(Self::key_up))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
                    .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_scroll_wheel(cx.listener(Self::scroll))
                    .child(TerminalElement::new(cx.entity())),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(terminal_color(self.palette.indexed(268)))
                    .child(format!(
                        "{}{}{}",
                        if self.bell { "● Bell · " } else { "" },
                        if self.clipboard_notice {
                            "Clipboard set by terminal · "
                        } else {
                            ""
                        },
                        if warning.is_empty() {
                            status
                        } else {
                            format!("{warning} · {status}")
                        }
                    )),
            )
    }
}

fn key_modifiers(modifiers: Modifiers) -> keys::Modifiers {
    keys::Modifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        control: modifiers.control,
        super_key: modifiers.platform,
    }
}

fn mouse_button(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        _ => 3,
    }
}

fn convert_key(stroke: &Keystroke, event: KeyEvent) -> Option<KeyInput> {
    let key = match stroke.key.as_str() {
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "escape" => Key::Escape,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "insert" => Key::Insert,
        "delete" => Key::Delete,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "space" => Key::Character(' '),
        key if key.starts_with('f') && key.len() <= 3 && key[1..].parse::<u8>().is_ok() => {
            Key::Function(key[1..].parse().ok()?)
        }
        key if key.chars().count() == 1 => Key::Character(key.chars().next()?),
        _ => return None,
    };
    Some(KeyInput {
        key,
        modifiers: key_modifiers(stroke.modifiers),
        event,
        text: stroke.key_char.clone(),
        shifted: stroke
            .key_char
            .as_ref()
            .filter(|_| stroke.modifiers.shift)
            .and_then(|s| s.chars().next()),
        base_layout: None,
    })
}

// Native text input owns only the current composition, not shell history.
// All offsets in this API are UTF-16. Committed text is sent once to the PTY;
// marked/preedit text stays local until AppKit commits or cancels it.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let (bytes, utf16) = crate::composition::range(&self.preedit, range);
        *adjusted = Some(utf16);
        Some(self.preedit[bytes].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.preedit_selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.preedit.is_empty()).then(|| 0..self.preedit.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.preedit.clear();
        self.preedit_selection = 0..0;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit.clear();
        self.preedit_selection = 0..0;
        self.write_input(text.as_bytes().to_vec());
        self.bell = false;
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range.unwrap_or(0..self.preedit.encode_utf16().count());
        let (bytes, replaced) = crate::composition::range(&self.preedit, range);
        self.preedit.replace_range(bytes, text);
        let end = text.encode_utf16().count();
        let selected = selected.unwrap_or(end..end);
        self.preedit_selection = (replaced.start + selected.start)..(replaced.start + selected.end);
        cx.notify();
    }
    fn paste(&mut self, item: ClipboardItem, _: &mut Window, _: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.paste_text(&text);
        }
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let cursor = self.snapshot.as_ref()?.cursor.as_ref()?;
        Some(Bounds::new(
            self.bounds.origin
                + point(
                    self.metrics.width * cursor.column as f32,
                    self.metrics.height * cursor.row as f32,
                ),
            size(self.metrics.width, self.metrics.height),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

fn exit_summary_id(snapshot: &RenderSnapshot) -> Option<String> {
    let mut text = String::new();
    let mut row = None;
    for run in &snapshot.runs {
        if row != Some(run.row) {
            text.push(' ');
            row = Some(run.row);
        }
        text.push_str(&run.text);
    }
    summary_id(&text)
}
fn summary_id(text: &str) -> Option<String> {
    let words: Vec<_> = text.split_whitespace().collect();
    let prefix = [
        "To", "continue", "this", "session,", "run", "codex", "resume",
    ];
    for start in (0..words.len().saturating_sub(7)).rev() {
        if words[start..start + 7] != prefix {
            continue;
        }
        // A narrow terminal can wrap the UUID across visible rows. Only gather
        // its exact 36 characters after the public exit-summary prefix.
        let mut id = String::new();
        for word in &words[start + 7..] {
            if id.len() + word.len() > 36
                || !word.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
            {
                break;
            }
            id.push_str(word);
            if id.len() == 36 {
                return Some(id);
            }
        }
    }
    None
}

#[cfg(test)]
mod key_routing_tests {
    use super::{SendBacktab, SendTab, key_bindings, summary_id};

    #[::core::prelude::v1::test]
    fn codex_exit_summary_id_handles_wrapping_and_rejects_other_text() {
        let id = "10000000-0000-4000-8000-000000000001";
        assert_eq!(
            summary_id(&format!("To continue this session, run codex resume {id}")),
            Some(id.into())
        );
        assert_eq!(
            summary_id(
                "To continue this session, run codex resume 10000000-0000-4000-\n8000-000000000001"
            ),
            Some(id.into())
        );
        assert!(summary_id(&format!("codex resume {id}")).is_none());
        assert!(
            summary_id("To continue this session, run codex resume 10000000; touch marker")
                .is_none()
        );
    }
    use gpui_kit::{KeyBinding, KeyContext, Keymap, Keystroke};

    #[::core::prelude::v1::test]
    fn terminal_tab_bypasses_root_navigation_but_inputs_keep_it() {
        let mut keymap = Keymap::default();
        // The root action's identity does not affect binding precedence.
        keymap.add_bindings([
            KeyBinding::new("tab", crate::Quit, Some("Root")),
            KeyBinding::new("shift-tab", crate::Quit, Some("Root")),
        ]);
        keymap.add_bindings(key_bindings());
        for key in ["tab", "shift-tab"] {
            let stroke = Keystroke::parse(key).unwrap();
            for (context, expected) in [("Terminal", 2), ("Input", 1)] {
                let (bindings, pending) = keymap.bindings_for_input(
                    std::slice::from_ref(&stroke),
                    &[
                        KeyContext::parse("Root").unwrap(),
                        KeyContext::parse(context).unwrap(),
                    ],
                );
                assert_eq!(bindings.len(), expected, "{key} in {context}");
                assert!(!pending);
                if context == "Terminal" {
                    assert!(if key == "tab" {
                        bindings[0].action().partial_eq(&SendTab)
                    } else {
                        bindings[0].action().partial_eq(&SendBacktab)
                    });
                }
            }
        }
    }
}
