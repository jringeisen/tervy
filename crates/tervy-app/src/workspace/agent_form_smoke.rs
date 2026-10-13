//! Native form/keyboard smoke; no agent CLI is invoked.
use super::*;

impl Workspace {
    pub(super) fn smoke_agent_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = "launch-fixture".to_string();
        self.projects = vec![Project {
            id: project.clone(),
            name: "Launch fixture".into(),
            path: std::env::temp_dir(),
            processes: vec![],
            terminals: vec![],
            agents: Default::default(),
        }];
        self.active_project = Some(project.clone());
        self.event(
            CoreEvent::SessionChanged(SessionInfo {
                id: SessionId(910001),
                launch: tervy_core::supervisor::LaunchId(0),
                project: project.clone(),
                name: "Saved Codex".into(),
                status: SessionStatus::Stopped,
                agent: Some(agent::Info {
                    record: agent::Record {
                        provider: agent::Provider::Codex,
                        launch_args: vec!["--approve-for-me".into()],
                        id: "saved-fixture".into(),
                        project: project.clone(),
                        name: "Saved Codex".into(),
                        cwd: std::env::temp_dir(),
                        role: agent::Role::Primary,
                        session_id: None,
                        parent: None,
                        snapshot_at: None,
                    },
                    status: agent::Status::Exited,
                }),
                process: None,
                config_changed: false,
                pid: None,
            }),
            window,
            cx,
        );
        self.show_agent_form(project, agent::Provider::Codex, false, None, window, cx);
        assert_eq!(
            self.agent_args_input.read(cx).value().as_ref(),
            "--approve-for-me"
        );
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, _cx| {
            window.resize(size(px(560.), px(480.)));
            window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |this, cx| {
                assert!(this.agent_launch.is_some());
                this.agent_args_input.update(cx, |input, cx| input.set_value("--model 'unfinished", window, cx));
            });
            window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, _| {
                    assert!(this.agent_launch.is_some());
                    assert!(this.agent_launch_error.is_some());
                    assert!(!this.tabs.values().any(|tab| tab.info.pid.is_some()));
                });
                window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
                window.on_next_frame(move |window, cx| {
                    let _ = weak.update(cx, |this, _| assert!(this.agent_launch.is_none()));
                    window.dispatch_keystroke(Keystroke::parse("cmd-b").unwrap(), cx);
                    window.on_next_frame(move |window, cx| {
                        let _ = weak.update(cx, |this, _| assert!(this.sidebar_collapsed));
                        window.dispatch_keystroke(Keystroke::parse("cmd-b").unwrap(), cx);
                        window.on_next_frame(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                assert!(!this.sidebar_collapsed);
                                this.focus_sidebar(&FocusSidebar, window, cx);
                            });
                            window.dispatch_keystroke(Keystroke::parse("cmd-b").unwrap(), cx);
                            window.on_next_frame(move |window, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    assert!(this.sidebar_collapsed);
                                    assert!(!this.sidebar_focus.contains_focused(window, cx));
                                });
                                window.dispatch_keystroke(Keystroke::parse("cmd-0").unwrap(), cx);
                                window.on_next_frame(move |window, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        assert!(!this.sidebar_collapsed);
                                        assert!(this.sidebar_focus.contains_focused(window, cx));
                                        assert_eq!(this.sidebar_width, 280.);
                                        assert!(!this.tabs.values().any(|tab| tab.info.pid.is_some()));
                                        eprintln!("tervy-agent-form-smoke PASS saved_flags=true malformed_args_block_launch=true escape_cancels=true sidebar_toggle=true focus_restored=true narrow_window=true");
                                        cx.quit();
                                    });
                                });
                            });
                        });
                    });
                });
            });
            });
        });
        cx.notify();
    }
}
