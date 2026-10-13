use super::*;
use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::base::Disableable;

impl Workspace {
    pub(super) fn search_notes(&mut self, cx: &mut Context<Self>) {
        if !self.note_search_input.read(cx).value().trim().is_empty() && !self.note_is_dirty(cx) {
            self.note_selected = None;
            self.note_loaded = None;
            self.note_new = false;
        }
        self.notes.clear();
        self.note_next_offset = None;
        self.note_searching = true;
        self.refresh_notes(cx);
        cx.notify();
    }

    pub(super) fn clear_note_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.note_search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search_notes(cx);
        self.note_search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn refresh_notes(&self, cx: &Context<Self>) {
        self.core.send(CoreCommand::ListNotes {
            query: self.note_search_input.read(cx).value().to_string(),
            limit: 100,
            offset: 0,
        });
    }

    pub(super) fn load_more_notes(&self, cx: &Context<Self>) {
        if let Some(offset) = self.note_next_offset {
            self.core.send(CoreCommand::ListNotes {
                query: self.note_search_input.read(cx).value().to_string(),
                limit: 100,
                offset,
            });
        }
    }

    pub(super) fn open_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.notes_open = true;
        self.notes_expanded = true;
        self.skills_open = false;
        self.meetings_open = false;
        self.palette_open = false;
        self.save_layout();
        self.refresh_notes(cx);
        self.update_visibility(window, cx);
        if self.note_is_dirty(cx) || self.note_saving {
            self.note_error = Some("Save or discard your changes before returning to Notes".into());
            cx.notify();
            return false;
        }
        self.note_selected = None;
        self.note_loaded = None;
        self.note_new = false;
        self.note_error = None;
        self.note_search_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        true
    }

    pub(super) fn note_is_dirty(&self, cx: &Context<Self>) -> bool {
        let title = self.note_title_input.read(cx).value();
        let body = self.note_body_input.read(cx).value();
        if self.note_new {
            return !title.is_empty() || !body.is_empty();
        }
        self.note_loaded
            .as_ref()
            .is_some_and(|note| title.as_ref() != note.title || body.as_ref() != note.body)
    }

    pub(super) fn load_note(
        &mut self,
        note: notes::Note,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.note_selected = Some(note.id.clone());
        self.note_title_input.update(cx, |input, cx| {
            input.set_value(note.title.clone(), window, cx)
        });
        self.note_body_input.update(cx, |input, cx| {
            input.set_value(note.body.clone(), window, cx)
        });
        self.note_loaded = Some(note);
        self.note_new = false;
        self.note_error = None;
        cx.notify();
    }

    pub(super) fn select_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_selected.as_ref() == Some(&id) && self.note_is_dirty(cx) {
            self.notes_open = true;
            self.skills_open = false;
            self.meetings_open = false;
            self.notes_expanded = true;
            self.update_visibility(window, cx);
            cx.notify();
            return;
        }
        if self.note_selected.as_ref() != Some(&id) && self.note_is_dirty(cx) {
            self.note_error =
                Some("Save or discard your changes before opening another note".into());
            cx.notify();
            return;
        }
        self.notes_open = true;
        self.skills_open = false;
        self.meetings_open = false;
        self.notes_expanded = true;
        self.note_selected = Some(id.clone());
        self.note_new = false;
        self.note_error = None;
        self.core.send(CoreCommand::ReadNote(id));
        self.update_visibility(window, cx);
        cx.notify();
    }

    pub(super) fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_is_dirty(cx) {
            self.note_error =
                Some("Save or discard your changes before creating another note".into());
            cx.notify();
            return;
        }
        self.notes_open = true;
        self.skills_open = false;
        self.meetings_open = false;
        self.notes_expanded = true;
        self.note_selected = None;
        self.note_loaded = None;
        self.note_new = true;
        self.note_error = None;
        self.note_title_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.note_body_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.update_visibility(window, cx);
        self.note_title_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn discard_note_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(note) = self.note_loaded.clone() {
            self.load_note(note, window, cx);
        } else {
            self.note_selected = None;
            self.note_new = false;
            self.note_title_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.note_body_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.note_error = None;
        }
        cx.notify();
    }

    pub(super) fn save_note(&mut self, cx: &mut Context<Self>) {
        if self.note_saving {
            return;
        }
        let write = notes::Write {
            id: self.note_loaded.as_ref().map(|note| note.id.clone()),
            title: self.note_title_input.read(cx).value().to_string(),
            body: self.note_body_input.read(cx).value().to_string(),
            expected_revision: self.note_loaded.as_ref().map(|note| note.revision),
        };
        if let Err(error) = write.validate() {
            self.note_error = Some(error);
        } else {
            self.note_saving = true;
            self.note_error = None;
            self.core.send(CoreCommand::WriteNote(write));
        }
        cx.notify();
    }

    pub(super) fn delete_selected_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = self.note_loaded.clone() else {
            return;
        };
        self.confirm_delete_note(note.id, note.revision, note.title, window, cx);
    }

    pub(super) fn confirm_delete_note(
        &mut self,
        id: String,
        revision: i64,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dialog_pending {
            return;
        }
        self.dialog_pending = true;
        let message = if self.note_selected.as_ref() == Some(&id) && self.note_is_dirty(cx) {
            format!("{title}\nYour unsaved changes will remain as a new draft.")
        } else {
            title
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete saved note?",
            Some(&message),
            &["Cancel", "Delete note"],
            cx,
        );
        self._tasks.push(cx.spawn(async move |view, cx| {
            let approved = answer.await == Ok(1);
            let _ = view.update(cx, |this, cx| {
                this.dialog_pending = false;
                if approved {
                    this.core.send(CoreCommand::DeleteNote { id, revision });
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn show_new_note_modal(
        &mut self,
        _: &NewNote,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shutdown.is_some() || self.note_modal_saving {
            return;
        }
        if self.note_modal_open {
            self.note_modal_title_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        self.palette_open = false;
        self.note_modal_open = true;
        self.note_modal_error = None;
        self.note_modal_title_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.note_modal_body_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub(super) fn cancel_new_note_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_modal_saving {
            return;
        }
        self.note_modal_open = false;
        self.note_modal_error = None;
        self.restore_focus(window, cx);
        cx.notify();
    }

    pub(super) fn save_new_note_modal(&mut self, cx: &mut Context<Self>) {
        if !self.note_modal_open || self.note_modal_saving {
            return;
        }
        if self.note_saving {
            self.note_modal_error = Some("Wait for the current note to finish saving".into());
            cx.notify();
            return;
        }
        let write = notes::Write {
            id: None,
            title: self.note_modal_title_input.read(cx).value().to_string(),
            body: self.note_modal_body_input.read(cx).value().to_string(),
            expected_revision: None,
        };
        if let Err(error) = write.validate() {
            self.note_modal_error = Some(error);
        } else {
            self.note_modal_saving = true;
            self.note_modal_error = None;
            self.core.send(CoreCommand::WriteNote(write));
        }
        cx.notify();
    }

    pub(super) fn new_note_modal(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut panel = dialog_panel(600., theme)
            .id("new-note-modal-panel")
            .max_h(px(
                (f32::from(window.viewport_size().height) - 96.).max(120.)
            ))
            .overflow_y_scroll()
            .child(div().font_semibold().child("New note"))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child("Title"))
                    .child(modal_input(&self.note_modal_title_input, "Note title")),
            )
            .child(
                v_flex().gap_2().child(div().text_sm().child("Note")).child(
                    Textarea::new(&self.note_modal_body_input)
                        .aria_label("Note body")
                        .h(px(220.)),
                ),
            );
        if let Some(error) = &self.note_modal_error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        panel = panel.child(
            h_flex()
                .gap_3()
                .child(
                    Button::new("save-new-note-modal")
                        .primary()
                        .disabled(self.note_modal_saving)
                        .label(if self.note_modal_saving {
                            "Saving…"
                        } else {
                            "Save note"
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.save_new_note_modal(cx))),
                )
                .child(
                    Button::new("cancel-new-note-modal")
                        .ghost()
                        .disabled(self.note_modal_saving)
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_new_note_modal(window, cx)
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
            .pt(px(64.))
            .child(panel)
            .into_any_element()
    }

    pub(super) fn notes_content(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = self.note_loaded.as_ref();
        let query = self.note_search_input.read(cx).value().to_string();
        let has_query = !query.trim().is_empty();
        let dirty = self.note_is_dirty(cx);
        let status = if self.note_searching {
            "Searching…".to_string()
        } else if self.note_next_offset.is_some() {
            format!("{}+ notes", self.notes.len())
        } else if self.notes.len() == 1 {
            "1 note".to_string()
        } else {
            format!("{} notes", self.notes.len())
        };
        let mut panel = v_flex()
            .id("notes-content-scroll")
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
                            .child(div().text_lg().font_semibold().child("Notes"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("A shared scratchpad for you and your Tervy agents."),
                            ),
                    )
                    .child(
                        Button::new("new-note-main")
                            .small()
                            .primary()
                            .icon(IconName::Plus)
                            .label("New")
                            .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                modal_input(&self.note_search_input, "Search saved notes")
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .when(has_query, |row| {
                                row.child(
                                    Button::new("clear-note-search")
                                        .small()
                                        .ghost()
                                        .label("Clear")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.clear_note_search(window, cx)
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
        if selected.is_some() || self.note_new {
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
                                .child(div().text_sm().font_semibold().child(if self.note_new {
                                    "New note"
                                } else {
                                    "Edit note"
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
                                        Button::new("copy-note")
                                            .small()
                                            .ghost()
                                            .icon(IconName::Copy)
                                            .tooltip("Copy note text")
                                            .accessibility_label("Copy note text")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    this.note_body_input
                                                        .read(cx)
                                                        .value()
                                                        .to_string(),
                                                ));
                                            })),
                                    )
                                    .child(
                                        Button::new("delete-note")
                                            .small()
                                            .ghost()
                                            .disabled(self.note_saving)
                                            .icon(AssetIconName::Trash)
                                            .label("Delete")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.delete_selected_note(window, cx)
                                            })),
                                    ),
                            )
                        }),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_xs().font_semibold().child("TITLE"))
                        .child(modal_input(&self.note_title_input, "Note title")),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_xs().font_semibold().child("PROMPT TEXT"))
                        .child(
                            Textarea::new(&self.note_body_input)
                                .aria_label("Note body")
                                .h(px(320.)),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("save-note")
                                .small()
                                .primary()
                                .disabled(self.note_saving || !dirty)
                                .label(if self.note_saving {
                                    "Saving…"
                                } else {
                                    "Save"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.save_note(cx))),
                        )
                        .child(
                            Button::new("discard-note")
                                .small()
                                .ghost()
                                .disabled(self.note_saving || (!self.note_new && !dirty))
                                .label(if self.note_new { "Cancel" } else { "Discard" })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.discard_note_changes(window, cx)
                                })),
                        ),
                );
            panel = panel.child(editor);
        } else {
            if self.notes.is_empty() {
                panel = panel.child(
                    v_flex()
                        .p_6()
                        .gap_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.muted)
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .child(if self.note_searching {
                                    "Searching notes…"
                                } else if has_query {
                                    "No matching notes"
                                } else {
                                    "Your note library is empty"
                                }),
                        )
                        .child(div().text_sm().text_color(theme.muted_foreground).child(
                            if has_query {
                                "Try another search or clear the filter."
                            } else {
                                "Capture ideas and working notes across projects."
                            },
                        )),
                );
            } else {
                let mut results = v_flex().gap_2();
                for note in &self.notes {
                    let id = note.id.clone();
                    let delete_id = id.clone();
                    let title = note.title.clone();
                    let revision = note.revision;
                    results =
                        results.child(
                            h_flex()
                                .gap_2()
                                .p_2()
                                .rounded_md()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.muted)
                                .child(
                                    Button::new(SharedString::from(format!("note-result-{id}")))
                                        .ghost()
                                        .flex_1()
                                        .min_w_0()
                                        .icon(IconName::FileText)
                                        .label(note.title.clone())
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.select_note(id.clone(), window, cx)
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "delete-note-result-{delete_id}"
                                    )))
                                    .small()
                                    .ghost()
                                    .icon(AssetIconName::Trash)
                                    .tooltip("Delete note")
                                    .accessibility_label("Delete note")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm_delete_note(
                                            delete_id.clone(),
                                            revision,
                                            title.clone(),
                                            window,
                                            cx,
                                        )
                                    })),
                                ),
                        );
                }
                if self.note_next_offset.is_some() {
                    results = results.child(
                        Button::new("more-notes-main")
                            .small()
                            .ghost()
                            .label("Load more notes")
                            .on_click(cx.listener(|this, _, _, cx| this.load_more_notes(cx))),
                    );
                }
                panel = panel.child(results);
            }
        }
        if let Some(error) = &self.note_error {
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
