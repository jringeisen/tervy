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
        self.rename = None;
        self.question_parent = None;
        self.customization_provider = None;
        self.customization_preview = None;
        self.palette_open = false;
        self.process_project = Some(project);
        self.process_error = None;
        self.process_name_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.process_command_input
            .update(cx, |input, cx| input.set_value("", window, cx));
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
            self.core.send(CoreCommand::AddProcess {
                project,
                name,
                command,
            });
        }
        cx.notify();
    }

    pub(super) fn cancel_process(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.process_saving {
            return;
        }
        self.process_project = None;
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
        let mut panel = v_flex()
            .w(px(480.))
            .h_auto()
            .p_4()
            .gap_3()
            .rounded_lg()
            .bg(cx.theme().background)
            .child(
                div()
                    .font_semibold()
                    .child(format!("Add process · {project}")),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Name"))
                    .child(Input::new(&self.process_name_input)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child("Command"))
                    .child(Input::new(&self.process_command_input)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Runs in the project folder. After adding, use its start button to run it.",
                    ),
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
                        .label(if self.process_saving {
                            "Adding…"
                        } else {
                            "Add process"
                        })
                        .disabled(self.process_saving)
                        .on_click(cx.listener(|this, _, _, cx| this.save_process(cx))),
                )
                .child(
                    Button::new("cancel-process")
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
            .bg(hsla(0., 0., 0., 0.3))
            .flex()
            .justify_center()
            .items_start()
            .pt(px(100.))
            .child(panel)
            .into_any_element()
    }
}
