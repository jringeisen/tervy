use super::*;
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::Disableable;

impl Workspace {
    pub(super) fn search_prompts(&mut self, cx: &mut Context<Self>) {
        if !self.prompt_search_input.read(cx).value().trim().is_empty() && !self.prompt_is_dirty(cx)
        {
            self.prompt_selected = None;
            self.prompt_loaded = None;
            self.prompt_new = false;
        }
        self.prompts.clear();
        self.prompt_next_offset = None;
        self.prompt_searching = true;
        self.refresh_prompts(cx);
        cx.notify();
    }

    pub(super) fn clear_prompt_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search_prompts(cx);
        self.prompt_search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn refresh_prompts(&self, cx: &Context<Self>) {
        self.core.send(CoreCommand::ListPrompts {
            query: self.prompt_search_input.read(cx).value().to_string(),
            limit: 100,
            offset: 0,
        });
    }

    pub(super) fn load_more_prompts(&self, cx: &Context<Self>) {
        if let Some(offset) = self.prompt_next_offset {
            self.core.send(CoreCommand::ListPrompts {
                query: self.prompt_search_input.read(cx).value().to_string(),
                limit: 100,
                offset,
            });
        }
    }

    pub(super) fn open_prompts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompts_open = true;
        self.prompts_expanded = true;
        self.meetings_open = false;
        self.palette_open = false;
        self.refresh_prompts(cx);
        self.update_visibility(window, cx);
        self.prompt_search_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn prompt_is_dirty(&self, cx: &Context<Self>) -> bool {
        let title = self.prompt_title_input.read(cx).value();
        let body = self.prompt_body_input.read(cx).value();
        if self.prompt_new {
            return !title.is_empty() || !body.is_empty();
        }
        self.prompt_loaded
            .as_ref()
            .is_some_and(|prompt| title.as_ref() != prompt.title || body.as_ref() != prompt.body)
    }

    pub(super) fn load_prompt(
        &mut self,
        prompt: prompts::Prompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_selected = Some(prompt.id.clone());
        self.prompt_title_input.update(cx, |input, cx| {
            input.set_value(prompt.title.clone(), window, cx)
        });
        self.prompt_body_input.update(cx, |input, cx| {
            input.set_value(prompt.body.clone(), window, cx)
        });
        self.prompt_loaded = Some(prompt);
        self.prompt_new = false;
        self.prompt_error = None;
        cx.notify();
    }

    pub(super) fn select_prompt(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.prompt_selected.as_ref() == Some(&id) && self.prompt_is_dirty(cx) {
            self.prompts_open = true;
            self.meetings_open = false;
            self.prompts_expanded = true;
            self.update_visibility(window, cx);
            cx.notify();
            return;
        }
        if self.prompt_selected.as_ref() != Some(&id) && self.prompt_is_dirty(cx) {
            self.prompt_error =
                Some("Save or discard your changes before opening another prompt".into());
            cx.notify();
            return;
        }
        self.prompts_open = true;
        self.meetings_open = false;
        self.prompts_expanded = true;
        self.prompt_selected = Some(id.clone());
        self.prompt_new = false;
        self.prompt_error = None;
        self.core.send(CoreCommand::ReadPrompt(id));
        self.update_visibility(window, cx);
        cx.notify();
    }

    pub(super) fn new_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt_is_dirty(cx) {
            self.prompt_error =
                Some("Save or discard your changes before creating another prompt".into());
            cx.notify();
            return;
        }
        self.prompts_open = true;
        self.meetings_open = false;
        self.prompts_expanded = true;
        self.prompt_selected = None;
        self.prompt_loaded = None;
        self.prompt_new = true;
        self.prompt_error = None;
        self.prompt_title_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.prompt_body_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.update_visibility(window, cx);
        self.prompt_title_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn discard_prompt_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(prompt) = self.prompt_loaded.clone() {
            self.load_prompt(prompt, window, cx);
        } else {
            self.prompt_selected = None;
            self.prompt_new = false;
            self.prompt_title_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.prompt_body_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.prompt_error = None;
        }
        cx.notify();
    }

    pub(super) fn save_prompt(&mut self, cx: &mut Context<Self>) {
        if self.prompt_saving {
            return;
        }
        let write = prompts::Write {
            id: self.prompt_loaded.as_ref().map(|prompt| prompt.id.clone()),
            title: self.prompt_title_input.read(cx).value().to_string(),
            body: self.prompt_body_input.read(cx).value().to_string(),
            expected_revision: self.prompt_loaded.as_ref().map(|prompt| prompt.revision),
        };
        if let Err(error) = write.validate() {
            self.prompt_error = Some(error);
        } else {
            self.prompt_saving = true;
            self.prompt_error = None;
            self.core.send(CoreCommand::WritePrompt(write));
        }
        cx.notify();
    }

    pub(super) fn delete_selected_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt_loaded.clone() else {
            return;
        };
        if self.prompt_pending {
            return;
        }
        self.prompt_pending = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete saved prompt?",
            Some(&prompt.title),
            &["Cancel", "Delete prompt"],
            cx,
        );
        self._tasks.push(cx.spawn(async move |view, cx| {
            let approved = answer.await == Ok(1);
            let _ = view.update(cx, |this, cx| {
                this.prompt_pending = false;
                if approved {
                    this.core.send(CoreCommand::DeletePrompt {
                        id: prompt.id,
                        revision: prompt.revision,
                    });
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn use_prompt_with_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.prompt_loaded.as_ref() else {
            return;
        };
        if self.prompt_is_dirty(cx) {
            self.prompt_error = Some("Save your changes before sending this prompt".into());
            cx.notify();
            return;
        }
        let Some(id) = self.active().filter(|id| {
            self.tabs.get(id).is_some_and(|tab| {
                tab.info.agent.is_some() && tab.info.status == SessionStatus::Running
            })
        }) else {
            self.prompt_error = Some("Select a running agent before using this prompt".into());
            cx.notify();
            return;
        };
        let body = prompt.body.clone();
        self.tabs[&id]
            .view
            .update(cx, |view, _| view.insert_text(&body));
        self.choose_tab(id, window, cx);
    }

    pub(super) fn prompts_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = self.prompt_loaded.as_ref();
        let query = self.prompt_search_input.read(cx).value().to_string();
        let has_query = !query.trim().is_empty();
        let dirty = self.prompt_is_dirty(cx);
        let status = if self.prompt_searching {
            "Searching…".to_string()
        } else if self.prompt_next_offset.is_some() {
            format!("{}+ prompts", self.prompts.len())
        } else if self.prompts.len() == 1 {
            "1 prompt".to_string()
        } else {
            format!("{} prompts", self.prompts.len())
        };
        let mut panel = v_flex()
            .id("prompts-content-scroll")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .gap_4()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(div().text_lg().font_semibold().child("Prompt library"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Available in every project and to your Tervy agents."),
                            ),
                    )
                    .child(
                        Button::new("new-prompt-main")
                            .small()
                            .primary()
                            .icon(IconName::Plus)
                            .label("New")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_prompt(window, cx)),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                modal_input(&self.prompt_search_input, "Search saved prompts")
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .when(has_query, |row| {
                                row.child(
                                    Button::new("clear-prompt-search")
                                        .small()
                                        .ghost()
                                        .label("Clear")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.clear_prompt_search(window, cx)
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(status),
                    ),
            );
        if selected.is_some() || self.prompt_new {
            let editor = v_flex()
                .min_w_0()
                .gap_4()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(theme.border)
                .bg(theme.muted)
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().font_semibold().child(if self.prompt_new {
                                    "New prompt"
                                } else {
                                    "Edit prompt"
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(if dirty { "Unsaved changes" } else { "Saved" }),
                                ),
                        )
                        .when(selected.is_some(), |header| {
                            header.child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new("copy-prompt")
                                            .small()
                                            .ghost()
                                            .icon(IconName::Copy)
                                            .tooltip("Copy prompt text")
                                            .accessibility_label("Copy prompt text")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    this.prompt_body_input
                                                        .read(cx)
                                                        .value()
                                                        .to_string(),
                                                ));
                                            })),
                                    )
                                    .child(
                                        Button::new("use-prompt-agent")
                                            .small()
                                            .ghost()
                                            .icon(AssetIconName::Send)
                                            .tooltip("Use with active agent")
                                            .accessibility_label("Use with active agent")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.use_prompt_with_agent(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("delete-prompt")
                                            .small()
                                            .ghost()
                                            .icon(AssetIconName::Trash)
                                            .tooltip("Delete prompt")
                                            .accessibility_label("Delete prompt")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.delete_selected_prompt(window, cx)
                                            })),
                                    ),
                            )
                        }),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_xs().font_semibold().child("TITLE"))
                        .child(modal_input(&self.prompt_title_input, "Prompt title")),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_xs().font_semibold().child("PROMPT TEXT"))
                        .child(
                            Textarea::new(&self.prompt_body_input)
                                .aria_label("Prompt body")
                                .h(px(320.)),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("save-prompt")
                                .small()
                                .primary()
                                .disabled(self.prompt_saving || !dirty)
                                .label(if self.prompt_saving {
                                    "Saving…"
                                } else {
                                    "Save"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.save_prompt(cx))),
                        )
                        .child(
                            Button::new("discard-prompt")
                                .small()
                                .ghost()
                                .disabled(self.prompt_saving || (!self.prompt_new && !dirty))
                                .label(if self.prompt_new { "Cancel" } else { "Discard" })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.discard_prompt_changes(window, cx)
                                })),
                        ),
                );
            panel = panel.child(editor);
        } else {
            if self.prompts.is_empty() {
                panel =
                    panel.child(
                        v_flex()
                            .p_6()
                            .gap_2()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.muted)
                            .child(div().text_sm().font_semibold().child(
                                if self.prompt_searching {
                                    "Searching prompts…"
                                } else if has_query {
                                    "No matching prompts"
                                } else {
                                    "Your prompt library is empty"
                                },
                            ))
                            .child(div().text_sm().text_color(theme.muted_foreground).child(
                                if has_query {
                                    "Try another search or clear the filter."
                                } else {
                                    "Save prompts here to reuse them across projects and agents."
                                },
                            )),
                    );
            } else {
                let mut results = v_flex().gap_2();
                for prompt in &self.prompts {
                    let id = prompt.id.clone();
                    results = results.child(
                        h_flex()
                            .p_2()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.muted)
                            .child(
                                Button::new(SharedString::from(format!("prompt-result-{id}")))
                                    .ghost()
                                    .icon(IconName::FileText)
                                    .label(prompt.title.clone())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select_prompt(id.clone(), window, cx)
                                    })),
                            ),
                    );
                }
                if self.prompt_next_offset.is_some() {
                    results = results.child(
                        Button::new("more-prompts-main")
                            .small()
                            .ghost()
                            .label("Load more prompts")
                            .on_click(cx.listener(|this, _, _, cx| this.load_more_prompts(cx))),
                    );
                }
                panel = panel.child(results);
            }
        }
        if let Some(error) = &self.prompt_error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        panel.into_any_element()
    }
}
