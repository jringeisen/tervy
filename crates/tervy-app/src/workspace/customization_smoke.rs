//! Isolated native UI smoke: no model calls or configuration writes.
use super::*;
impl Workspace {
    pub(super) fn smoke_customization(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.perf_layout = "customization-running".into();
        let mut config = self.config.clone();
        config.settings.theme = "warm-dark".into();
        config.settings.font_size = 18.;
        config.settings.sidebar_width = Some(240.);
        self.event(CoreEvent::Config(config), window, cx);
        self.customization_preview = Some(tervy_core::customization::Preview {
            proposal: "[settings]\ntheme = \"warm-dark\"\nfont_size = 18\nsidebar_width = 240"
                .into(),
            changes: "− font_size = 14\n+ font_size = 18".into(),
        });
        cx.notify();
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |this, cx| {
                assert!(this.customization_preview.is_some());
                assert!(Theme::global(cx).is_dark());
                assert_eq!(crate::theme::terminal_palette(cx).background.r, 0x28);
                assert_eq!(this.sidebar_width, 240.);
                this.customization_preview = None;
                this.customization_provider = Some(agent::Provider::Claude);
                this.rename_input.update(cx, |input, cx| {
                    input.set_value("Make my workspace warmer", window, cx);
                    input.focus(window, cx);
                });
                cx.notify();
                let next_view = cx.weak_entity();
                window.on_next_frame(move |window, cx| {
                    let _ = next_view.update(cx, |this, cx| {
                        let project = this.projects.first().expect("smoke project").id.clone();
                        this.show_process_form(project, window, cx);
                        this.save_process(cx);
                        assert_eq!(this.process_error.as_deref(), Some("Enter a process name"));
                        assert!(!this.process_saving);
                        window.on_next_frame(move |_, cx| {
                            eprintln!("tervy-customization-smoke PASS preview_and_prompt_rendered=true warm_theme=true sidebar=240 process_form_rendered=true empty_name_rejected=true");
                            cx.quit();
                        });
                    });
                });
            });
        });
    }
}
