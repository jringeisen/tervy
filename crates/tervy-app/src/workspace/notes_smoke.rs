//! Native Notes navigation and draft regression without touching live data.
use super::*;

impl Workspace {
    pub(super) fn smoke_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.click_sidebar_row(SidebarRow::Notes, window, cx);
        assert!(self.notes_open && !self.skills_open);
        assert!(
            self.note_search_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        let id = "0f81a940-0a97-4e6f-a7aa-d81f33325701".to_string();
        let note = notes::Note {
            id: id.clone(),
            title: "Draft idea".into(),
            body: "Original".into(),
            revision: 1,
            updated_at: 1,
        };
        self.event(
            CoreEvent::NotesListed {
                query: String::new(),
                offset: 0,
                items: vec![notes::Summary {
                    id: id.clone(),
                    title: note.title.clone(),
                    revision: 1,
                    updated_at: 1,
                }],
            },
            window,
            cx,
        );
        assert!(self.sidebar_rows().contains(&SidebarRow::Note(id.clone())));
        self.click_sidebar_row(SidebarRow::Note(id.clone()), window, cx);
        self.event(
            CoreEvent::NoteRead {
                id,
                note: Some(note.clone()),
            },
            window,
            cx,
        );
        self.note_body_input
            .update(cx, |input, cx| input.set_value("Local draft", window, cx));
        assert!(self.note_is_dirty(cx));
        self.event(
            CoreEvent::NoteSaved {
                note: notes::Note {
                    body: "Agent edit".into(),
                    revision: 2,
                    ..note
                },
                from_ui: false,
            },
            window,
            cx,
        );
        assert_eq!(
            self.note_body_input.read(cx).value().as_ref(),
            "Local draft"
        );
        assert!(self.note_error.is_some());
        self.click_sidebar_row(SidebarRow::Notes, window, cx);
        assert!(self.note_loaded.is_some() && self.note_is_dirty(cx));
        self.discard_note_changes(window, cx);
        assert!(!self.note_is_dirty(cx));
        self.click_sidebar_row(SidebarRow::Notes, window, cx);
        assert!(self.note_loaded.is_none() && self.note_selected.is_none());
        assert_eq!(self.sidebar_cursor, Some(SidebarRow::Notes));
        self.new_note(window, cx);
        self.note_title_input
            .update(cx, |input, cx| input.set_value("New idea", window, cx));
        assert!(self.note_is_dirty(cx));
        self.click_sidebar_row(SidebarRow::Notes, window, cx);
        assert!(self.note_new && self.note_error.is_some());
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            window.dispatch_keystroke(Keystroke::parse("cmd-n").unwrap(), cx);
            window.on_next_frame(move |window, cx| {
                window.resize(size(px(560.), px(480.)));
                let _ = weak.update(cx, |this, cx| {
                    assert!(this.note_modal_open, "Cmd-N did not open note modal");
                    assert!(
                        this.note_modal_title_input
                            .read(cx)
                            .focus_handle(cx)
                            .is_focused(window)
                    );
                    this.note_modal_title_input
                        .update(cx, |input, cx| input.set_value("Shortcut note", window, cx));
                    this.note_modal_body_input
                        .update(cx, |input, cx| input.set_value("Separate draft", window, cx));
                    assert_eq!(this.note_title_input.read(cx).value().as_ref(), "New idea");
                    this.cancel_new_note_modal(window, cx);
                    assert!(!this.note_modal_open && this.note_new && this.note_is_dirty(cx));
                    eprintln!("tervy-notes-smoke PASS sidebar=true parent_navigation=true draft_guard=true drafts=true conflict=true modal=true narrow_window=true");
                    cx.quit();
                });
            });
        });
        cx.notify();
    }
}
