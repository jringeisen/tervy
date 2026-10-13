use super::*;
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::Disableable;

impl Workspace {
    pub(super) fn search_skills(&mut self, cx: &mut Context<Self>) {
        if !self.skill_search_input.read(cx).value().trim().is_empty() && !self.skill_is_dirty(cx) {
            self.skill_selected = None;
            self.skill_loaded = None;
            self.skill_new = false;
        }
        self.skill_items.clear();
        self.skill_source_counts.clear();
        self.skill_next_offset = None;
        self.skill_searching = true;
        self.refresh_skills(cx);
        cx.notify();
    }

    pub(super) fn refresh_skills(&self, cx: &Context<Self>) {
        self.core.send(CoreCommand::ListSkills {
            query: self.skill_search_input.read(cx).value().to_string(),
            limit: 100,
            offset: 0,
        });
    }

    pub(super) fn load_more_skills(&self, cx: &Context<Self>) {
        if let Some(offset) = self.skill_next_offset {
            self.core.send(CoreCommand::ListSkills {
                query: self.skill_search_input.read(cx).value().to_string(),
                limit: 100,
                offset,
            });
        }
    }

    pub(super) fn open_skills(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let dirty = self.skill_is_dirty(cx) || self.skill_file_is_dirty(cx);
        self.skills_open = true;
        self.skills_expanded = true;
        self.notes_open = false;
        self.meetings_open = false;
        self.palette_open = false;
        self.save_layout();
        self.refresh_skills(cx);
        self.update_visibility(window, cx);
        if dirty || self.skill_saving || self.skill_file_saving {
            self.skill_error =
                Some("Save or discard your changes before returning to Skills".into());
            cx.notify();
            return false;
        }
        self.skill_selected = None;
        self.skill_loaded = None;
        self.skill_new = false;
        self.skill_file_path = None;
        self.skill_file_revision = None;
        self.skill_file_saved_content.clear();
        self.skill_error = None;
        self.skill_search_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        true
    }

    pub(super) fn skill_is_dirty(&self, cx: &Context<Self>) -> bool {
        let name = self.skill_name_input.read(cx).value();
        let content = self.skill_content_input.read(cx).value();
        if self.skill_new {
            return !name.is_empty() || !content.is_empty();
        }
        self.skill_loaded
            .as_ref()
            .is_some_and(|skill| content.as_ref() != skill.content)
    }

    pub(super) fn skill_file_is_dirty(&self, cx: &Context<Self>) -> bool {
        self.skill_file_path.is_some()
            && self.skill_file_input.read(cx).value().as_ref() != self.skill_file_saved_content
    }

    fn select_skill_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.skill_file_is_dirty(cx) {
            self.skill_error =
                Some("Save or discard the supporting file before opening another".into());
            cx.notify();
            return;
        }
        let Some(id) = self.skill_selected.clone() else {
            return;
        };
        self.skill_file_path = Some(path.clone());
        self.skill_file_revision = None;
        self.skill_file_saved_content.clear();
        self.skill_file_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.core.send(CoreCommand::ReadSkillFile { id, path });
        cx.notify();
    }

    fn new_skill_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.skill_file_is_dirty(cx) {
            self.skill_error = Some("Save or discard the current supporting file first".into());
            cx.notify();
            return;
        }
        let path = self
            .skill_file_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if path.is_empty() {
            self.skill_error =
                Some("Enter a relative file path such as references/notes.md".into());
            cx.notify();
            return;
        }
        self.skill_file_path = Some(path);
        self.skill_file_revision = None;
        self.skill_file_saved_content.clear();
        self.skill_file_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn save_skill_file(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(path)) = (self.skill_selected.clone(), self.skill_file_path.clone())
        else {
            return;
        };
        self.skill_file_saving = true;
        self.skill_error = None;
        self.core
            .send(CoreCommand::WriteSkillFile(skills::FileWrite {
                id,
                path,
                content: self.skill_file_input.read(cx).value().to_string(),
                expected_revision: self.skill_file_revision.clone(),
            }));
        cx.notify();
    }

    fn discard_skill_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.skill_file_input.update(cx, |input, cx| {
            input.set_value(self.skill_file_saved_content.clone(), window, cx)
        });
        self.skill_error = None;
        cx.notify();
    }

    pub(super) fn load_skill(
        &mut self,
        skill: skills::Skill,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.skill_selected = Some(skill.summary.id.clone());
        self.skill_source = skill.summary.source.clone();
        self.skill_name_input.update(cx, |input, cx| {
            input.set_value(skill.summary.name.clone(), window, cx)
        });
        self.skill_content_input.update(cx, |input, cx| {
            input.set_value(skill.content.clone(), window, cx)
        });
        self.skill_loaded = Some(skill);
        self.skill_file_path = None;
        self.skill_file_revision = None;
        self.skill_file_saved_content.clear();
        self.skill_new = false;
        self.skill_error = None;
        cx.notify();
    }

    pub(super) fn select_skill(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.skill_selected.as_ref() == Some(&id)
            && (self.skill_is_dirty(cx) || self.skill_file_is_dirty(cx))
        {
            self.skills_open = true;
            self.update_visibility(window, cx);
            cx.notify();
            return;
        }
        if self.skill_selected.as_ref() != Some(&id)
            && (self.skill_is_dirty(cx) || self.skill_file_is_dirty(cx))
        {
            self.skill_error =
                Some("Save or discard your changes before opening another skill".into());
            cx.notify();
            return;
        }
        self.skills_open = true;
        self.meetings_open = false;
        self.notes_open = false;
        self.skills_expanded = true;
        self.skill_selected = Some(id.clone());
        self.skill_new = false;
        self.skill_error = None;
        self.core.send(CoreCommand::ReadSkill(id));
        self.update_visibility(window, cx);
        cx.notify();
    }

    pub(super) fn new_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.skill_is_dirty(cx) || self.skill_file_is_dirty(cx) {
            self.skill_error =
                Some("Save or discard your changes before creating another skill".into());
            cx.notify();
            return;
        }
        self.skills_open = true;
        self.notes_open = false;
        self.meetings_open = false;
        self.skills_expanded = true;
        self.skill_selected = None;
        self.skill_loaded = None;
        self.skill_file_path = None;
        self.skill_new = true;
        self.skill_error = None;
        self.skill_name_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.skill_content_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.update_visibility(window, cx);
        self.skill_name_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn discard_skill_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(skill) = self.skill_loaded.clone() {
            self.load_skill(skill, window, cx);
        } else {
            self.skill_selected = None;
            self.skill_new = false;
            self.skill_name_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.skill_content_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.skill_error = None;
        }
        cx.notify();
    }

    pub(super) fn save_skill(&mut self, cx: &mut Context<Self>) {
        if self.skill_saving {
            return;
        }
        let write = skills::Write {
            id: self
                .skill_loaded
                .as_ref()
                .map(|skill| skill.summary.id.clone()),
            source: self.skill_source.clone(),
            name: self.skill_name_input.read(cx).value().to_string(),
            content: self.skill_content_input.read(cx).value().to_string(),
            expected_revision: self
                .skill_loaded
                .as_ref()
                .map(|skill| skill.summary.revision.clone()),
        };
        self.skill_saving = true;
        self.skill_error = None;
        self.core.send(CoreCommand::WriteSkill(write));
        cx.notify();
    }

    pub(super) fn delete_selected_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(skill) = self.skill_loaded.clone() else {
            return;
        };
        if skill.summary.read_only || self.dialog_pending {
            return;
        }
        self.dialog_pending = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete skill and its files?",
            Some(&skill.summary.name),
            &["Cancel", "Delete skill"],
            cx,
        );
        self._tasks.push(cx.spawn(async move |view, cx| {
            let approved = answer.await == Ok(1);
            let _ = view.update(cx, |this, cx| {
                this.dialog_pending = false;
                if approved {
                    this.core.send(CoreCommand::DeleteSkill {
                        id: skill.summary.id,
                        revision: skill.summary.revision,
                    });
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn skills_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = self.skill_loaded.as_ref();
        let dirty = self.skill_is_dirty(cx);
        let mut panel = v_flex()
            .id("skills-content-scroll")
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
                            .gap_1()
                            .child(div().text_lg().font_semibold().child("Skills"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Global skills used by your agents."),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("refresh-skills")
                                    .small()
                                    .ghost()
                                    .label("Refresh")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.refresh_skills(cx)),
                                    ),
                            )
                            .child(
                                Button::new("new-skill-main")
                                    .small()
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label("New")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_skill(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        modal_input(&self.skill_search_input, "Search skills")
                            .flex_1()
                            .min_w_0(),
                    )
                    .when(
                        !self.skill_search_input.read(cx).value().is_empty(),
                        |row| {
                            row.child(
                                Button::new("clear-skill-search")
                                    .small()
                                    .ghost()
                                    .label("Clear")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.skill_search_input.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                        this.search_skills(cx);
                                    })),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} skills", self.skill_items.len())),
            );

        if selected.is_some() || self.skill_new {
            let read_only = selected.is_some_and(|skill| skill.summary.read_only);
            let mut header = h_flex().items_center().justify_between().child(
                div().text_sm().font_semibold().child(if self.skill_new {
                    "New skill"
                } else {
                    "Skill editor"
                }),
            );
            if selected.is_some() {
                let mut actions = h_flex().gap_1().child(
                    Button::new("copy-skill")
                        .small()
                        .ghost()
                        .icon(IconName::Copy)
                        .tooltip("Copy SKILL.md")
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                this.skill_content_input.read(cx).value().to_string(),
                            ));
                        })),
                );
                if !read_only {
                    actions = actions.child(
                        Button::new("delete-skill")
                            .small()
                            .ghost()
                            .icon(AssetIconName::Trash)
                            .tooltip("Delete skill")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.delete_selected_skill(window, cx)
                            })),
                    );
                }
                header = header.child(actions);
            }
            let mut editor =
                v_flex()
                    .min_w_0()
                    .gap_4()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.muted)
                    .child(header)
                    .child(div().text_xs().text_color(theme.muted_foreground).child(
                        if read_only {
                            "Managed skill · read only"
                        } else if dirty {
                            "Unsaved changes"
                        } else {
                            "Saved"
                        },
                    ));
            if self.skill_new {
                editor = editor.child(div().text_xs().font_semibold().child("AGENT / SOURCE"));
                let mut sources = h_flex().gap_2().flex_wrap();
                for source in skills::Catalog::current().sources() {
                    let source_id = source.id.clone();
                    let mut button = Button::new(SharedString::from(format!("source-{source_id}")))
                        .small()
                        .label(source.agent);
                    if self.skill_source == source_id {
                        button = button.primary();
                    }
                    sources = sources.child(button.on_click(cx.listener(move |this, _, _, cx| {
                        this.skill_source = source_id.clone();
                        cx.notify();
                    })));
                }
                editor = editor.child(sources)
                    .child(div().text_xs().font_semibold().child("FOLDER NAME"))
                    .child(modal_input(&self.skill_name_input, "skill-name"))
                    .child(Button::new("skill-template").small().ghost().label("Insert SKILL.md template")
                        .on_click(cx.listener(|this, _, window, cx| {
                            let name = this.skill_name_input.read(cx).value().to_string();
                            if name.is_empty() { this.skill_error = Some("Enter a folder name first".into()); return; }
                            this.skill_content_input.update(cx, |input, cx| input.set_value(
                                format!("---\nname: {name}\ndescription: Describe when this skill should be used.\n---\n\n# Instructions\n\n"), window, cx));
                            cx.notify();
                        })));
            } else if let Some(skill) = selected {
                editor = editor.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("{} · {}", skill.summary.agent, skill.summary.id)),
                );
            }
            editor = editor
                .child(div().text_xs().font_semibold().child("SKILL.MD"))
                .child(
                    Textarea::new(&self.skill_content_input)
                        .aria_label("SKILL.md content")
                        .disabled(read_only)
                        .h(px(360.)),
                )
                .when(!read_only, |editor| {
                    editor.child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("save-skill")
                                    .small()
                                    .primary()
                                    .disabled(self.skill_saving || !dirty)
                                    .label(if self.skill_saving {
                                        "Saving…"
                                    } else {
                                        "Save"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.save_skill(cx))),
                            )
                            .child(
                                Button::new("discard-skill")
                                    .small()
                                    .ghost()
                                    .disabled(self.skill_saving || (!self.skill_new && !dirty))
                                    .label(if self.skill_new { "Cancel" } else { "Discard" })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.discard_skill_changes(window, cx)
                                    })),
                            ),
                    )
                });
            if let Some(skill) = selected {
                editor = editor.child(div().text_xs().font_semibold().child("SUPPORTING FILES"));
                for file in &skill.files {
                    let path = file.clone();
                    editor = editor.child(
                        Button::new(SharedString::from(format!("skill-file-{path}")))
                            .small()
                            .ghost()
                            .label(file.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_skill_file(path.clone(), window, cx)
                            })),
                    );
                }
                if !read_only {
                    editor = editor.child(
                        h_flex()
                            .gap_2()
                            .child(
                                modal_input(&self.skill_file_name_input, "references/notes.md")
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .child(
                                Button::new("new-skill-file")
                                    .small()
                                    .ghost()
                                    .label("New file")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_skill_file(window, cx)
                                    })),
                            ),
                    );
                }
                if let Some(path) = &self.skill_file_path {
                    let file_dirty = self.skill_file_is_dirty(cx);
                    editor = editor
                        .child(div().text_xs().font_semibold().child(path.clone()))
                        .child(
                            Textarea::new(&self.skill_file_input)
                                .aria_label("Supporting file text")
                                .disabled(read_only)
                                .h(px(240.)),
                        );
                    if !read_only {
                        editor = editor.child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("save-skill-file")
                                        .small()
                                        .primary()
                                        .disabled(self.skill_file_saving || !file_dirty)
                                        .label("Save file")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.save_skill_file(cx)),
                                        ),
                                )
                                .child(
                                    Button::new("discard-skill-file")
                                        .small()
                                        .ghost()
                                        .disabled(self.skill_file_saving || !file_dirty)
                                        .label("Discard")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.discard_skill_file(window, cx)
                                        })),
                                ),
                        );
                    }
                }
            }
            panel = panel.child(editor);
        } else {
            if self.skill_items.is_empty() {
                panel = panel.child(
                    div()
                        .p_6()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.muted)
                        .child(if self.skill_searching {
                            "Loading skills…"
                        } else {
                            "No skills found"
                        }),
                );
            } else {
                let mut results = v_flex().gap_2();
                for skill in &self.skill_items {
                    let id = skill.id.clone();
                    results = results.child(
                        Button::new(SharedString::from(format!("skill-result-{id}")))
                            .ghost()
                            .icon(IconName::FileText)
                            .label(format!("{} · {}", skill.agent, skill.name))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_skill(id.clone(), window, cx)
                            })),
                    );
                }
                if self.skill_next_offset.is_some() {
                    results = results.child(
                        Button::new("more-skills")
                            .small()
                            .ghost()
                            .label("Load more")
                            .on_click(cx.listener(|this, _, _, cx| this.load_more_skills(cx))),
                    );
                }
                panel = panel.child(results);
            }
        }
        if let Some(error) = &self.skill_error {
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
