//! Click-to-refocus regression with an actual terminal view behind the dialogs.
use super::*;
use std::sync::{Mutex, OnceLock};

fn bounds() -> &'static Mutex<BTreeMap<&'static str, Bounds<Pixels>>> {
    static BOUNDS: OnceLock<Mutex<BTreeMap<&'static str, Bounds<Pixels>>>> = OnceLock::new();
    BOUNDS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn measure(
    name: &'static str,
    index: usize,
) -> impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) {
    move |children, _, _| {
        if let Some(child) = children.get(index) {
            bounds().lock().unwrap().insert(name, *child);
        }
    }
}

fn click(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            position,
            button: MouseButton::Left,
            click_count: 1,
            ..Default::default()
        }),
        cx,
    );
}

fn refocus(view: WeakEntity<Workspace>, case: usize, point_index: usize, window: &mut Window) {
    window.on_next_frame(move |window, _| {
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| this.focus.focus(window, cx));
            let name = match case { 0 => "Agent launch arguments", 1 => "Process name", 2 => "Command", _ => "Rename" };
            let field = bounds().lock().unwrap()[name];
            let center = field.center();
            let positions = [center,
                point(field.origin.x + px(2.), center.y),
                point(field.origin.x + field.size.width - px(2.), center.y),
                point(center.x, field.origin.y + px(2.)),
                point(center.x, field.origin.y + field.size.height - px(2.)),
            ];
            let position = positions[point_index];
            click(position, window, cx);
            window.on_next_frame(move |window, cx| {
                let input = view.update(cx, |this, _| match case {
                    0 => this.agent_args_input.clone(),
                    1 => this.process_name_input.clone(),
                    2 => this.process_command_input.clone(),
                    _ => this.rename_input.clone(),
                }).unwrap();
                let before = input.read(cx).value().len();
                window.dispatch_keystroke(Keystroke::parse("x").unwrap(), cx);
                assert_eq!(input.read(cx).value().len(), before + 1, "modal input {case} did not accept typing at point {point_index}");
                let _ = view.update(cx, |this, cx| {
                    let input = match case {
                        0 => &this.agent_args_input,
                        1 => &this.process_name_input,
                        2 => &this.process_command_input,
                        _ => &this.rename_input,
                    };
                    assert!(input.read(cx).focus_handle(cx).is_focused(window), "modal input {case} did not regain focus after click");
                    assert!(!this.tabs[&SessionId(920001)].view.read(cx).focus.is_focused(window), "underlying terminal took focus");
                    if point_index < 4 {
                        refocus(view.clone(), case, point_index + 1, window);
                        return;
                    }
                    match case {
                        0 => {
                            this.cancel_agent_form(window, cx);
                            this.show_process_form("focus-fixture".into(), window, cx);
                        }
                        1 => {}
                        2 => {
                            this.cancel_process(window, cx);
                            this.rename = Some(SessionId(920001));
                            this.rename_input.update(cx, |input, cx| { input.set_value("Fixture terminal", window, cx); input.focus(window, cx); });
                            cx.notify();
                        }
                        _ => {
                            eprintln!("tervy-modal-focus-smoke PASS agent_arguments=true process_name=true process_command=true rename=true terminal_does_not_steal_focus=true full_field_clicks=20 typing_checks=20");
                            cx.quit();
                            return;
                        }
                    }
                    refocus(view.clone(), case + 1, 0, window);
                });
            });
        });
    });
}

impl Workspace {
    pub(super) fn smoke_modal_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.projects = vec![Project {
            id: "focus-fixture".into(),
            name: "Focus fixture".into(),
            path: std::env::temp_dir(),
            processes: vec![],
            terminals: vec![],
            agents: Default::default(),
        }];
        self.active_project = Some("focus-fixture".into());
        self.event(
            CoreEvent::SessionChanged(SessionInfo {
                id: SessionId(920001),
                launch: tervy_core::supervisor::LaunchId(0),
                project: "focus-fixture".into(),
                name: "Fixture terminal".into(),
                status: SessionStatus::Stopped,
                agent: None,
                process: None,
                config_changed: false,
                pid: None,
            }),
            window,
            cx,
        );
        self.choose_tab(SessionId(920001), window, cx);
        window.resize(size(px(800.), px(600.)));
        self.show_agent_form(
            "focus-fixture".into(),
            agent::Provider::Codex,
            false,
            None,
            window,
            cx,
        );
        refocus(cx.weak_entity(), 0, 0, window);
    }
}
