//! Native M5 split presentation smoke probe. Only synthetic UI mirrors.
use super::*;

impl Workspace {
    /// Opt-in native render probe: synthetic mirrors only, no CLI or project
    /// process is launched. The real actor lifecycle is covered separately.
    pub(super) fn smoke_side(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = if std::env::var("DEVDESK_PERF_PROVIDER").as_deref() == Ok("codex") {
            agent::Provider::Codex
        } else {
            agent::Provider::Claude
        };
        let parent = SessionId(10_001);
        let side = SessionId(10_002);
        let primary = agent::Record {
            id: "10000000-0000-4000-8000-000000000001".into(),
            project: "probe".into(),
            name: "Synthetic parent".into(),
            cwd: std::env::temp_dir(),
            role: agent::Role::Primary,
            launch_args: vec![],
            provider,
            session_id: Some("20000000-0000-4000-8000-000000000002".into()),
            parent: None,
            snapshot_at: None,
        };
        for (id, record) in [
            (parent, primary.clone()),
            (
                side,
                agent::Record {
                    id: "30000000-0000-4000-8000-000000000003".into(),
                    name: "Why this choice?".into(),
                    role: agent::Role::Side,
                    launch_args: vec![],
                    provider,
                    parent: Some(primary.id.clone()),
                    snapshot_at: Some(1_791_518_400),
                    ..primary
                },
            ),
        ] {
            self.event(
                CoreEvent::SessionChanged(SessionInfo {
                    agent: Some(agent::Info {
                        record,
                        status: agent::Status::Idle,
                    }),
                    process: None,
                    config_changed: false,
                    pid: None,
                    id,
                    launch: devdesk_core::supervisor::LaunchId(1),
                    project: "probe".into(),
                    name: if id == parent {
                        "Synthetic parent"
                    } else {
                        "Synthetic side"
                    }
                    .into(),
                    status: SessionStatus::Running,
                }),
                window,
                cx,
            );
        }
        self.active_project = Some("probe".into());
        self.update_visibility(window, cx);
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |this, cx| {
                let left = this.tabs[&parent].view.read(cx).bounds;
                let right = this.tabs[&side].view.read(cx).bounds;
                assert!(left.size.width > px(150.) && right.size.width > px(150.));
                assert!(right.origin.x > left.origin.x);
                // Compare actual native shaping metrics with the terminal's
                // cell mask, rather than repeating the height calculation.
                let terminal = this.tabs[&parent].view.read(cx);
                let cell_height = terminal.metrics.height;
                for (bold, italic) in [(false, false), (true, false), (false, true)] {
                    let mut face = font(terminal.font_family.clone());
                    face.features = FontFeatures::disable_ligatures();
                    if bold { face = face.bold(); }
                    if italic { face = face.italic(); }
                    let text = "gyjpq";
                    let line = window.text_system().shape_line(text.into(), px(terminal.font_size), &[TextRun {
                        len: text.len(), font: face, color: black(), background_color: None,
                        underline: None, strikethrough: None,
                    }], None);
                    assert!(cell_height >= line.ascent + line.descent + px(2.),
                        "terminal cell clips native font: height={cell_height:?}, ascent={:?}, descent={:?}", line.ascent, line.descent);
                }
                eprintln!("devdesk-font-smoke PASS native_regular_bold_italic_fit=true cell_height_px={:.1}", f32::from(cell_height));
                assert_eq!(this.project_tabs(), vec![parent]);
                assert_eq!(this.visible_ids(), vec![parent, side]);
                this.close_terminal(&CloseTerminal, window, cx);
                assert_eq!(this.active(), Some(parent));
                assert!(this.side_panes.is_empty());
                assert!(!this.tabs[&side].open);
                let weak = cx.weak_entity();
                window.on_next_frame(move |window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let width = this.tabs[&parent].view.read(cx).bounds.size.width;
                        assert!(width > left.size.width * 1.5);
                        this.error = Some("Select a running agent with a known session ID".into());
                        this.ask_agent_question(&AskAgentQuestion, window, cx);
                        assert_eq!(this.question_parent, Some(parent));
                        assert!(this.error.is_none(), "successful question must clear an earlier error");
                        this.question_parent = None;
                        this.choose_tab(side, window, cx);
                        assert_eq!(this.visible_ids(), vec![parent, side]);
                        eprintln!("devdesk-side-smoke PASS split_px={:.1}/{:.1} parent_after_close_px={:.1} primary_retained=true history_reopened=true question_input=true", f32::from(left.size.width), f32::from(right.size.width), f32::from(width));
                        this.keyboard_smoke(false, window, cx);
                    });
                });
            });
        });
        cx.notify();
    }
    /// Dispatch registered shortcuts after releasing the Workspace entity lease.
    /// The native command component handles Enter, then restores pane focus.
    fn keyboard_smoke(&self, side: bool, window: &mut Window, cx: &Context<Self>) {
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            window.dispatch_keystroke(Keystroke::parse("cmd-p").unwrap(), cx);
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    assert!(this.palette_open);
                    let entries = this.palette_actions(cx);
                    assert!(entries.iter().any(|(_, action)| matches!(action, PaletteAction::Remove(RemovalTarget::Project(_)))));
                    assert!(entries.iter().any(|(_, action)| matches!(action, PaletteAction::Remove(RemovalTarget::Session(_)))));
                    assert!(entries.iter().any(|(_, action)| matches!(action, PaletteAction::Select(_))));
                    let query = if side { "Focus Side Conversation" } else { "Focus Primary Terminal" };
                    this.palette.update(cx, |state, cx| state.set_query(query, window, cx));
                });
                window.on_next_frame(move |window, _cx| {
                    // Query callbacks are deferred; render the resulting model.
                    window.on_next_frame(move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            assert_eq!(this.palette_actions(cx).len(), 1);
                        });
                        window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
                        window.on_next_frame(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                assert!(!this.palette_open);
                                let target = if side { this.side_focus.unwrap() } else { this.active().unwrap() };
                                assert!(this.tabs[&target].view.read(cx).focus.is_focused(window));
                                if side {
                                    eprintln!("devdesk-keyboard-smoke PASS palette_shortcut=true enter_confirmation=true primary_and_side_focus=true removal_and_history_available=true");
                                    let before = this.tabs[&target].view.read(cx).bounds.size.width;
                                    this.confirm_palette(PaletteAction::ResizeSide(50), window, cx);
                                    let resize_view = cx.weak_entity();
                                    window.on_next_frame(move |window, cx| {
                                        let _ = resize_view.update(cx, |this, cx| {
                                            let after = this.tabs[&target].view.read(cx).bounds.size.width;
                                            assert!(after >= before + px(49.));
                                            eprintln!("devdesk-resize-smoke PASS side_before_px={:.1} side_after_px={:.1}", f32::from(before), f32::from(after));
                                            this.sidebar_smoke(window, cx);
                                        });
                                    });
                                } else { this.question_shortcut_smoke(window, cx); }
                            });
                        });
                    });
                    window.refresh();
                });
            });
            window.refresh();
        });
        window.refresh();
    }
}

impl Workspace {
    fn question_shortcut_smoke(&self, window: &mut Window, cx: &Context<Self>) {
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            window.dispatch_keystroke(Keystroke::parse("cmd-shift-a").unwrap(), cx);
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    assert_eq!(this.question_parent, this.active());
                    assert!(this.question_parent.is_some() && this.error.is_none());
                    eprintln!("devdesk-question-shortcut-smoke PASS running_agent=true known_session=true cmd_shift_a_opens_question=true");
                    this.close_terminal(&CloseTerminal, window, cx);
                    this.keyboard_smoke(true, window, cx);
                });
            });
        });
        window.refresh();
    }

    /// Drive native pointer events across separate paints so the handle gets
    /// the same down/move/up lifecycle as a real drag. Dispatch happens after
    /// releasing the Workspace lease because input can synchronously redraw.
    fn sidebar_smoke(&self, window: &mut Window, cx: &Context<Self>) {
        let weak = cx.weak_entity();
        let before = self.sidebar_state.read(cx).sizes()[0];
        let x = self.tabs[&self.active().unwrap()]
            .view
            .read(cx)
            .bounds
            .origin
            .x;
        window.on_next_frame(move |window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: point(before, px(300.)), pressed_button: None, modifiers: Default::default() }), cx);
            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent {
                position: point(before, px(300.)), button: MouseButton::Left,
                modifiers: Default::default(), click_count: 1, first_mouse: false,
            }), cx);
            window.on_next_frame(move |window, cx| {
                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(before + px(60.), px(300.)),
                    pressed_button: Some(MouseButton::Left), modifiers: Default::default(),
                }), cx);
                window.on_next_frame(move |window, cx| {
                    // The first move starts GPUI's drag; a subsequent move
                    // drives the panel after that state has been painted.
                    window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent {
                        position: point(before + px(60.), px(300.)),
                        pressed_button: Some(MouseButton::Left), modifiers: Default::default(),
                    }), cx);
                    window.on_next_frame(move |window, cx| {
                    window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent {
                        position: point(before + px(60.), px(300.)), button: MouseButton::Left,
                        modifiers: Default::default(), click_count: 1,
                    }), cx);
                    window.on_next_frame(move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            let after = this.sidebar_state.read(cx).sizes()[0];
                            assert!(after >= before + px(59.), "sidebar drag failed: {before:?} -> {after:?}");
                            assert_eq!(this.sidebar_width, f32::from(after));
                            let content_x = this.tabs[&this.active().unwrap()].view.read(cx).bounds.origin.x;
                            assert!(content_x >= x + px(59.));
                            this.confirm_palette(PaletteAction::ResizeSidebar(1000), window, cx);
                            assert_eq!(this.sidebar_state.read(cx).sizes()[0], px(520.));
                            this.confirm_palette(PaletteAction::ResizeSidebar(-1000), window, cx);
                            assert_eq!(this.sidebar_state.read(cx).sizes()[0], px(220.));
                            assert_eq!(this.sidebar_width, 220.);
                            eprintln!("devdesk-sidebar-smoke PASS pointer_drag=true terminal_layout_updated=true remembered_width=true keyboard_resize=true range=220..520");
                            this.appearance_smoke(window, cx);
                        });
                    });
                    });
                });
            });
        });
        window.refresh();
    }

    /// Toggle the component theme without changing the user's macOS settings.
    /// Waiting for a frame lets every terminal's global observer update before
    /// checking palettes, including terminals that are currently hidden.
    fn appearance_smoke(&self, window: &mut Window, cx: &Context<Self>) {
        use gpui_kit::component::{Theme, ThemeMode};
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            Theme::change(ThemeMode::Light, Some(window), cx);
            crate::theme::apply_palette(cx);
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    let expected = devdesk_term::snapshot::Palette::light();
                    assert_eq!(cx.theme().background, terminal_surface(&expected));
                    for tab in this.tabs.values() {
                        assert!(tab.view.read(cx).palette == expected);
                    }
                });
                Theme::change(ThemeMode::Dark, Some(window), cx);
                crate::theme::apply_palette(cx);
                window.on_next_frame(move |window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let expected = devdesk_term::snapshot::Palette::default();
                        assert_eq!(cx.theme().background, terminal_surface(&expected));
                        for tab in this.tabs.values() {
                            assert!(tab.view.read(cx).palette == expected);
                        }
                        eprintln!("devdesk-appearance-smoke PASS light_then_dark=true visible_and_hidden_terminal_palettes=true");
                        this.narrow_window_smoke(window, cx);
                    });
                });
            });
        });
        window.refresh();
    }

    fn narrow_window_smoke(&self, window: &mut Window, cx: &Context<Self>) {
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, _cx| {
            window.resize(size(px(560.), px(480.)));
            window.on_next_frame(move |window, _cx| {
                window.on_next_frame(move |window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let sidebar = this.sidebar_state.read(cx).sizes()[0];
                        assert!(sidebar <= px(240.));
                        assert!(!this.sidebar_has_memory_column(cx));
                        let bounds = this.tabs[&this.active().unwrap()].view.read(cx).bounds;
                        assert!(bounds.size.width >= px(300.), "narrow primary bounds: {bounds:?}");
                        let side = this.side_panes[&this.active().unwrap()];
                        let side_bounds = this.tabs[&side].view.read(cx).bounds;
                        assert!(side_bounds.origin.y > bounds.origin.y);
                        assert!(side_bounds.size.width >= px(300.));
                        assert!(bounds.origin.x + bounds.size.width <= window.viewport_size().width);
                        eprintln!("devdesk-narrow-smoke PASS viewport=560x480 sidebar_fits=true terminal_visible=true memory_in_tooltip=true");
                        cx.quit();
                    });
                });
            });
        });
        window.refresh();
    }
}

fn terminal_surface(palette: &devdesk_term::snapshot::Palette) -> Hsla {
    let color = palette.background;
    rgb((u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)).into()
}
