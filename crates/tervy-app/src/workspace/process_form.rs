use super::*;
use gpui_kit::base::Disableable;

impl Workspace {
    pub(super) fn show_process_form(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.process_saving {
            return;
        }
        self.setup_open = false;
        self.agent_launch = None;
        self.rename = None;
        self.question_parent = None;
        self.customization_provider = None;
        self.customization_preview = None;
        self.palette_open = false;
        self.process_project = Some(project);
        self.process_edit = None;
        self.process_autostart = false;
        self.process_error = None;
        self.process_name_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.process_command_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub(super) fn show_edit_process_form(
        &mut self,
        session: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.process_saving {
            return;
        }
        let Some(info) = self.tabs.get(&session).map(|tab| &tab.info) else {
            return;
        };
        let Some(process_id) = info.process.as_ref() else {
            return;
        };
        let Some(spec) = self
            .projects
            .iter()
            .find(|project| project.id == info.project)
            .and_then(|project| {
                project
                    .processes
                    .iter()
                    .find(|process| &process.id == process_id)
            })
            .cloned()
        else {
            self.error = Some("Process is no longer configured".into());
            return;
        };
        let project = info.project.clone();
        let process_id = process_id.clone();
        self.show_process_form(project, window, cx);
        self.process_edit = Some(process_id);
        self.process_autostart = spec.autostart;
        self.process_name_input.update(cx, |input, cx| {
            input.set_value(spec.name, window, cx);
            input.focus(window, cx);
        });
        self.process_command_input.update(cx, |input, cx| {
            input.set_value(spec.command, window, cx);
        });
        cx.notify();
    }

    pub(super) fn save_process(&mut self, cx: &mut Context<Self>) {
        if self.process_saving {
            return;
        }
        let Some(project) = self.process_project.clone() else {
            return;
        };
        let name = self.process_name_input.read(cx).value().trim().to_string();
        let command = self
            .process_command_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        self.process_error = if name.is_empty() {
            Some("Enter a process name".into())
        } else if command.is_empty() {
            Some("Enter a command to run".into())
        } else {
            None
        };
        if self.process_error.is_none() {
            self.process_saving = true;
            if let Some(process) = self.process_edit.clone() {
                self.core.send(CoreCommand::EditProcess {
                    project,
                    process,
                    name,
                    command,
                    autostart: self.process_autostart,
                });
            } else {
                self.core.send(CoreCommand::AddProcess {
                    project,
                    name,
                    command,
                });
            }
        }
        cx.notify();
    }

    pub(super) fn cancel_process(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.process_saving {
            return;
        }
        self.process_project = None;
        self.process_edit = None;
        self.process_error = None;
        self.restore_focus(window, cx);
        cx.notify();
    }

    pub(super) fn process_form(&self, cx: &Context<Self>) -> AnyElement {
        let project = self
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.process_project.as_ref())
            .map(|p| p.name.as_str())
            .unwrap_or("project");
        let mut panel = dialog_panel(480., cx.theme())
            .child(div().font_semibold().child(format!(
                "{} process · {project}",
                if self.process_edit.is_some() {
                    "Edit"
                } else {
                    "Add"
                }
            )))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child("Name"))
                    .when(self.perf_layout == "modal-focus", |group| {
                        group.on_children_prepainted(modal_focus_smoke::measure("Process name", 1))
                    })
                    .child(modal_input(&self.process_name_input, "Process name")),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child("Command"))
                    .when(self.perf_layout == "modal-focus", |group| {
                        group.on_children_prepainted(modal_focus_smoke::measure("Command", 1))
                    })
                    .child(modal_input(&self.process_command_input, "Command")),
            )
            .when(self.process_edit.is_some(), |panel| {
                panel.child(
                    Button::new("process-autostart")
                        .ghost()
                        .label(if self.process_autostart {
                            "Autostart: On"
                        } else {
                            "Autostart: Off"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.process_autostart = !this.process_autostart;
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.process_edit.is_some() {
                        "Changes to a running command apply when you restart the process."
                    } else {
                        "Runs in the project folder. Add it now, then choose Start when ready."
                    }),
            );
        if let Some(error) = &self.process_error {
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
                    Button::new("save-process")
                        .primary()
                        .loading(self.process_saving)
                        .label(if self.process_saving {
                            "Saving…"
                        } else {
                            if self.process_edit.is_some() {
                                "Save process"
                            } else {
                                "Add process"
                            }
                        })
                        .disabled(self.process_saving)
                        .on_click(cx.listener(|this, _, _, cx| this.save_process(cx))),
                )
                .child(
                    Button::new("cancel-process")
                        .ghost()
                        .label("Cancel")
                        .disabled(self.process_saving)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.cancel_process(window, cx)),
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
