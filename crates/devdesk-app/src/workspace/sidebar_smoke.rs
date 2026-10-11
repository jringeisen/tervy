//! Native keyboard navigation smoke using stopped synthetic sessions.
use super::*;

impl Workspace {
    pub(super) fn smoke_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.perf_layout = "sidebar-running".into();
        self.projects = (0..20)
            .map(|i| Project {
                id: format!("navigation-{i}"),
                name: format!("Navigation project {i}"),
                path: std::env::temp_dir(),
                processes: Vec::new(),
                terminals: Vec::new(),
                agents: Default::default(),
            })
            .collect();
        self.active_project = Some("navigation-0".into());
        self.expanded.clear();
        self.prompts_expanded = false;
        self.meetings_expanded = false;
        // Stopped synthetic sessions exercise the real rows without launching commands.
        for (id, process, open) in [
            (900003, None, true),
            (900002, Some("server"), false),
            (900004, None, false),
        ] {
            self.event(
                CoreEvent::SessionChanged(SessionInfo {
                    id: SessionId(id),
                    launch: devdesk_core::supervisor::LaunchId(0),
                    project: "navigation-1".into(),
                    name: format!("Navigation session {id}"),
                    status: SessionStatus::Stopped,
                    agent: None,
                    process: process.map(str::to_owned),
                    config_changed: false,
                    pid: None,
                }),
                window,
                cx,
            );
            self.tabs.get_mut(&SessionId(id)).unwrap().open = open;
        }
        self.sidebar_cursor = None;
        self.focus_sidebar(&FocusSidebar, window, cx);
        self.sidebar_cursor = Some(SidebarRow::Project("navigation-0".into()));
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            window.resize(size(px(560.), px(480.)));
            window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx);
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    assert_eq!(this.sidebar_cursor, Some(SidebarRow::Project("navigation-1".into())));
                    assert_eq!(this.active_project.as_deref(), Some("navigation-0"));
                    assert!(this.sidebar_focus.contains_focused(window, cx));
                });
                window.dispatch_keystroke(Keystroke::parse("right").unwrap(), cx);
                window.on_next_frame(move |window, cx| {
                    let _ = weak.update(cx, |this, _| {
                        assert!(this.expanded.contains("navigation-1"));
                        assert_eq!(&this.sidebar_rows()[3..7], &[
                            SidebarRow::Project("navigation-1".into()),
                            SidebarRow::Session(SessionId(900002)),
                            SidebarRow::Session(SessionId(900003)),
                            SidebarRow::Project("navigation-2".into()),
                        ]);
                    });
                    window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx);
                    window.on_next_frame(move |window, cx| {
                        window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
                        window.on_next_frame(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                assert_eq!(this.active(), Some(SessionId(900002)));
                                assert!(this.tabs[&SessionId(900002)].view.read(cx).focus.is_focused(window));
                            });
                            window.dispatch_keystroke(Keystroke::parse("cmd-0").unwrap(), cx);
                            window.on_next_frame(move |window, cx| {
                                // Left from a session returns to its project, then collapses it.
                                window.dispatch_keystroke(Keystroke::parse("left").unwrap(), cx);
                                window.on_next_frame(move |window, cx| {
                                    window.dispatch_keystroke(Keystroke::parse("left").unwrap(), cx);
                                    window.on_next_frame(move |window, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            assert!(!this.expanded.contains("navigation-1"));
                                            for _ in 0..20 { this.move_sidebar(1, window, cx); }
                                        });
                                        window.on_next_frame(move |window, cx| {
                                            let _ = weak.update(cx, |this, _| {
                                                assert_eq!(this.sidebar_cursor, Some(SidebarRow::Project("navigation-19".into())));
                                                assert!(this.sidebar_scroll.offset().y < px(0.), "highlighted row did not scroll into view");
                                            });
                                            window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
                                            window.on_next_frame(move |window, cx| {
                                                let _ = weak.update(cx, |this, cx| {
                                                    assert!(!this.sidebar_focus.contains_focused(window, cx));
                                                    assert!(this.tabs[&SessionId(900002)].view.read(cx).focus.is_focused(window));
                                                    eprintln!("tervy-sidebar-smoke PASS arrows=true enter_focuses_terminal=true expand_collapse=true skips_closed_shells=true autoscroll=true escape_returns_focus=true narrow_window=true");
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
            });
        });
        cx.notify();
    }
}
