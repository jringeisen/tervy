use super::*;
use gpui_kit::base::Disableable;

impl Workspace {
    pub(super) fn smoke_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.readiness = Some(crate::setup::Readiness {
            shell: true,
            agents: vec![],
            warning: None,
        });
        self.setup_open = true;
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, _cx| {
            window.resize(size(px(560.), px(480.)));
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    assert!(this.setup_open);
                    assert!(this.readiness.as_ref().unwrap().shell);
                    assert!(!this.palette_actions(cx).iter().any(|(_, action)| matches!(action, PaletteAction::LaunchAgent(_, _) | PaletteAction::Customize(_))));
                    assert!(this.palette_actions(cx).iter().any(|(_, action)| matches!(action, PaletteAction::Setup)));
                });
                window.dispatch_keystroke(Keystroke::parse("escape").unwrap(), cx);
                window.on_next_frame(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        assert!(!this.setup_open);
                        eprintln!("devdesk-setup-smoke PASS setup_rendered=true optional_agents_not_required=true escape_dismisses=true narrow_window=true");
                        cx.quit();
                    });
                });
            });
        });
        cx.notify();
    }

    pub(super) fn dismiss_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Diagnostic windows must never acknowledge the real user's welcome.
        if self.perf_layout.is_empty()
            && let Err(error) =
                crate::setup::acknowledge_welcome(&devdesk_core::config::support_directory())
        {
            tracing::warn!(%error, "Could not save welcome dismissal");
        }
        self.setup_open = false;
        self.restore_focus(window, cx);
        cx.notify();
    }

    pub(super) fn check_setup(&mut self, cx: &mut Context<Self>) {
        if self.checking_setup {
            return;
        }
        self.checking_setup = true;
        let check = cx
            .background_executor()
            .spawn(async { crate::setup::inspect() });
        self._tasks.push(cx.spawn(async move |view, cx| {
            let readiness = check.await;
            let _ = view.update(cx, |this, cx| {
                this.readiness = Some(readiness);
                this.checking_setup = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn setup_summary(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut summary = v_flex()
            .w_full()
            .max_w(px(360.))
            .gap_2()
            .pt_4()
            .border_t_1()
            .border_color(theme.border);
        if let Some(readiness) = &self.readiness {
            for (name, available, optional) in std::iter::once(("Shell", readiness.shell, false))
                .chain(
                    agent::Provider::ALL.into_iter().map(|provider| {
                        (provider.label(), readiness.agents.contains(&provider), true)
                    }),
                )
            {
                summary = summary.child(
                    h_flex()
                        .gap_3()
                        .child(div().flex_1().text_sm().child(name))
                        .child(
                            div()
                                .text_xs()
                                .text_color(if available || optional {
                                    theme.muted_foreground
                                } else {
                                    theme.danger
                                })
                                .child(if available {
                                    "Installed"
                                } else if optional {
                                    "Optional · not found"
                                } else {
                                    "Not found"
                                }),
                        ),
                );
            }
        } else {
            summary = summary.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Checking available tools…"),
            );
        }
        summary
            .child(div().text_xs().text_color(theme.muted_foreground).child(
                "Start with a terminal. Coding agents are optional and use your own CLI accounts.",
            ))
            .into_any_element()
    }

    pub(super) fn setup_dialog(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut content = v_flex()
            .id("setup-content")
            .gap_4()
            .min_h_0()
            .max_h(px((f32::from(window.viewport_size().height) - 160.).max(120.)))
            .overflow_y_scroll()
            .child(div().text_lg().font_semibold().child("Welcome to Tervy"))
            .child(div().text_sm().text_color(theme.muted_foreground)
                .child("Keep your tools together. Keep your agents moving."))
            .child(v_flex().gap_3()
                .child(welcome_step("Projects", "Use + beside Projects to add a local folder. Switch projects in the sidebar; each keeps its own sessions."))
                .child(welcome_step("Terminals", "Use + beside Terminals to open a shell in your project. Your terminal sessions stay open when you switch projects."))
                .child(welcome_step("Processes", "Use + beside Processes to save and run project commands, like a dev server. Start, stop, and restart them from the sidebar."))
                .child(welcome_step("Agents", "Use + beside Agents to choose an installed agent: Claude Code, Codex, Gemini CLI, OpenCode, or Aider. Claude and Codex also support read-only side questions.")))
            .child(v_flex().gap_2()
                .child(div().text_sm().font_semibold().child("Your keyboard shortcuts"))
                .child(welcome_shortcut("⌘P", "Search all commands, including Setup and Prerequisites"))
                .child(welcome_shortcut("⌘0", "Focus the sidebar; use ↑/↓ to highlight and Enter to open"))
                .child(welcome_shortcut("⌘O", "Add a project"))
                .child(welcome_shortcut("⌘T", "Open a terminal"))
                .child(welcome_shortcut("⌘W", "Close the current tab")))
            .child(self.setup_summary(cx))
            .child(div().text_sm().text_color(theme.muted_foreground)
                .child("Install and sign in to your agent through its CLI, then use Recheck to find it. Restart Tervy if the installation changed your shell PATH."));
        if let Some(readiness) = &self.readiness {
            if !readiness.shell {
                content = content.child(div().text_sm().text_color(theme.danger)
                    .child("Your configured shell could not be found. Check your shell installation before opening a terminal."));
            }
            if let Some(warning) = &readiness.warning {
                content = content.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(warning.clone()),
                );
            }
        }
        let panel = dialog_panel(560., theme)
            .id("setup-panel")
            .child(content)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .child(
                        Button::new("setup-add-project")
                            .primary()
                            .label("Add project")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_project(&AddProject, window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("setup-recheck")
                            .ghost()
                            .label("Recheck")
                            .loading(self.checking_setup)
                            .disabled(self.checking_setup)
                            .on_click(cx.listener(|this, _, _, cx| this.check_setup(cx))),
                    )
                    .child(
                        Button::new("setup-done")
                            .ghost()
                            .label("Get started")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dismiss_setup(window, cx);
                            })),
                    ),
            );
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(hsla(0., 0., 0., 0.3))
            .flex()
            .justify_center()
            .items_start()
            .p_4()
            .pt_6()
            .child(panel)
            .into_any_element()
    }
}

fn welcome_step(title: &'static str, description: &'static str) -> Div {
    v_flex()
        .gap_1()
        .child(div().text_sm().font_semibold().child(title))
        .child(div().text_sm().child(description))
}

fn welcome_shortcut(keys: &'static str, description: &'static str) -> Div {
    h_flex()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(px(36.))
                .flex_shrink_0()
                .text_sm()
                .font_semibold()
                .child(keys),
        )
        .child(div().flex_1().min_w_0().text_sm().child(description))
}
