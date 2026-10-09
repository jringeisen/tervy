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
                        this.ask_agent_question(&AskAgentQuestion, window, cx);
                        assert_eq!(this.question_parent, Some(parent));
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
                                    window.on_next_frame(move |_, cx| {
                                        let _ = resize_view.update(cx, |this, cx| {
                                            let after = this.tabs[&target].view.read(cx).bounds.size.width;
                                            assert!(after >= before + px(49.));
                                            eprintln!("devdesk-resize-smoke PASS side_before_px={:.1} side_after_px={:.1}", f32::from(before), f32::from(after));
                                            cx.quit();
                                        });
                                    });
                                } else { this.keyboard_smoke(true, window, cx); }
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
