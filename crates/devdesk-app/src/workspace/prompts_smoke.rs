//! Native editor/search regression without reading the user's prompt database.
use super::*;

impl Workspace {
    pub(super) fn smoke_prompts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_project = Some("probe".into());
        self.click_sidebar_row(SidebarRow::Prompts, window, cx);
        assert!(self.prompts_open);
        assert!(
            self.prompt_search_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        self.prompts = vec![prompts::Summary {
            id: "00000000-0000-4000-8000-000000000002".into(),
            title: "Other prompt".into(),
            revision: 1,
            updated_at: 1,
        }];

        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            window.resize(size(px(560.), px(480.)));
            window.dispatch_keystroke(Keystroke::parse("m").unwrap(), cx);
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                assert_eq!(this.prompt_search_input.read(cx).value().as_ref(), "m");
                assert!(this.prompts.is_empty());
                let id = "00000000-0000-4000-8000-000000000001".to_string();
                this.event(
                    CoreEvent::PromptsListed {
                        query: "m".into(),
                        offset: 0,
                        items: vec![prompts::Summary {
                            id: id.clone(), title: "Minimal design".into(),
                            revision: 1, updated_at: 1,
                        }],
                    },
                    window,
                    cx,
                );
                assert_eq!(this.prompts.len(), 1);
                assert!(!this.prompt_searching);

                this.select_prompt(id.clone(), window, cx);
                this.event(
                    CoreEvent::PromptRead {
                        id: id.clone(),
                        prompt: Some(prompts::Prompt {
                            id, title: "Minimal design".into(),
                            body: "Use generous spacing.".into(),
                            revision: 1, updated_at: 1,
                        }),
                    },
                    window,
                    cx,
                );
                this.prompt_title_input.update(cx, |input, cx| {
                    input.set_value("Changed title", window, cx)
                });
                this.prompt_body_input.update(cx, |input, cx| {
                    input.set_value("Unsaved body", window, cx)
                });
                assert!(this.prompt_is_dirty(cx));
                this.discard_prompt_changes(window, cx);
                assert_eq!(this.prompt_title_input.read(cx).value().as_ref(), "Minimal design");
                assert_eq!(this.prompt_body_input.read(cx).value().as_ref(), "Use generous spacing.");
                assert!(!this.prompt_is_dirty(cx));
                this.prompt_search_input.update(cx, |input, cx| {
                    input.set_value("spacing", window, cx)
                });
                this.search_prompts(cx);
                assert!(this.prompt_selected.is_none());
                assert!(this.prompt_loaded.is_none());

                this.new_prompt(window, cx);
                this.prompt_title_input.update(cx, |input, cx| {
                    input.set_value("New draft", window, cx)
                });
                this.discard_prompt_changes(window, cx);
                assert!(!this.prompt_new);
                assert!(this.prompt_selected.is_none());
                assert!(this.prompt_title_input.read(cx).value().is_empty());
                this.clear_prompt_search(window, cx);
                assert!(this.prompt_search_input.read(cx).value().is_empty());
                });
                window.on_next_frame(move |_, cx| {
                    eprintln!("tervy-prompts-smoke PASS search_focus=true typed_query=true results=true discard_saved=true cancel_draft=true clear_search=true narrow_window=true");
                    cx.quit();
                });
            });
        });
        cx.notify();
    }
}
