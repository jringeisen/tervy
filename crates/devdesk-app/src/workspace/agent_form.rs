use super::*;

pub(super) struct AgentLaunch {
    project: String,
    provider: agent::Provider,
    picker: bool,
    session: Option<SessionId>,
}

impl Workspace {
    pub(super) fn show_resume_agent_form(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(info) = self.tabs.get(&id).and_then(|tab| tab.info.agent.as_ref()) {
            self.show_agent_form(
                info.record.project.clone(),
                info.record.provider,
                false,
                Some(id),
                window,
                cx,
            );
        }
    }

    pub(super) fn show_agent_form(
        &mut self,
        project: String,
        provider: agent::Provider,
        picker: bool,
        session: Option<SessionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shutdown.is_some() {
            return;
        }
        if let Some(id) = session
            && self
                .tabs
                .get(&id)
                .and_then(|tab| tab.info.agent.as_ref())
                .is_some_and(|info| info.record.role == agent::Role::Side)
        {
            self.core.send(CoreCommand::ResumeAgent(id));
            return;
        }
        let args = session
            .and_then(|id| self.tabs.get(&id))
            .and_then(|tab| tab.info.agent.as_ref())
            .or_else(|| {
                self.ordered_agent_ids()
                    .into_iter()
                    .rev()
                    .filter_map(|id| self.tabs[&id].info.agent.as_ref())
                    .find(|info| {
                        info.record.provider == provider && info.record.role == agent::Role::Primary
                    })
            })
            .map(|info| agent::format_launch_args(&info.record.launch_args))
            .unwrap_or_default();
        self.setup_open = false;
        self.palette_open = false;
        self.process_project = None;
        self.rename = None;
        self.question_parent = None;
        self.customization_provider = None;
        self.customization_preview = None;
        self.agent_launch = Some(AgentLaunch {
            project,
            provider,
            picker,
            session,
        });
        self.agent_launch_error = None;
        self.agent_args_input.update(cx, |input, cx| {
            input.set_value(args, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn launch_agent_from_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent_launch.is_none() || self.shutdown.is_some() {
            return;
        }
        let args = match agent::parse_launch_args(&self.agent_args_input.read(cx).value()) {
            Ok(args) => args,
            Err(error) => {
                self.agent_launch_error = Some(error);
                cx.notify();
                return;
            }
        };
        let launch = self.agent_launch.take().unwrap();
        self.agent_launch_error = None;
        self.active_project = Some(launch.project.clone());
        self.expanded.insert(launch.project.clone());
        if let Some(id) = launch.session {
            self.choose_tab(id, window, cx);
            self.core.send(CoreCommand::ResumeAgentWithArgs(id, args));
        } else {
            self.core.send(CoreCommand::NewAgentWithArgs {
                project: launch.project,
                provider: launch.provider,
                picker: launch.picker,
                args,
            });
        }
        self.update_visibility(window, cx);
        self.save_layout();
        self.restore_focus(window, cx);
        cx.notify();
    }

    pub(super) fn cancel_agent_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agent_launch = None;
        self.agent_launch_error = None;
        self.restore_focus(window, cx);
        cx.notify();
    }

    pub(super) fn agent_form(&self, cx: &Context<Self>) -> AnyElement {
        let launch = self.agent_launch.as_ref().unwrap();
        let provider = launch.provider;
        let project = self
            .projects
            .iter()
            .find(|project| project.id == launch.project)
            .map(|project| project.name.as_str())
            .unwrap_or("project");
        let verb = if launch.session.is_some() {
            if provider.tracks_sessions() {
                "Resume"
            } else {
                "Restart"
            }
        } else {
            "Launch"
        };
        let mut panel = dialog_panel(560., cx.theme())
            .child(div().font_semibold().child(format!("{verb} {} · {project}", provider.label())))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                if provider.tracks_sessions() {
                    "Integrated resume and read-only side questions. CLI sign-in is not checked here."
                } else {
                    "Basic interactive terminal. Resume and side questions are not supported. CLI sign-in is not checked here."
                }
            ))
            .child(v_flex().gap_2()
                .when(self.perf_layout == "modal-focus", |group| group.on_children_prepainted(modal_focus_smoke::measure("Agent launch arguments", 1)))
                .child(div().text_sm().child("Arguments"))
                .child(modal_input(&self.agent_args_input, "Agent launch arguments")))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child("Add flags and quote values containing spaces. These arguments are saved with the session and reused on resume or restart."))
            .child(div().text_sm().font_family(cx.theme().mono_font_family.clone()).child(format!("{} {}", provider.command(), self.agent_args_input.read(cx).value())));
        if let Some(error) = &self.agent_launch_error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        panel = panel.child(
            h_flex()
                .gap_3()
                .child(
                    Button::new("launch-agent-confirm")
                        .primary()
                        .label(verb)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.launch_agent_from_form(window, cx)
                        })),
                )
                .child(
                    Button::new("launch-agent-cancel")
                        .ghost()
                        .label("Cancel")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.cancel_agent_form(window, cx)),
                        ),
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
            .pt(px(64.))
            .child(panel)
            .into_any_element()
    }
}
