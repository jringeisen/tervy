//! Native skill editor/search regression without reading the user's skill files.
use super::*;

impl Workspace {
    pub(super) fn smoke_skills(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_project = Some("probe".into());
        self.click_sidebar_row(SidebarRow::Skills, window, cx);
        assert!(self.skills_open);
        assert!(
            self.skill_search_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        self.skill_items = vec![skills::Summary {
            id: "claude:other".into(),
            source: "claude".into(),
            agent: "Claude".into(),
            name: "other".into(),
            description: "Other skill".into(),
            revision: "a".into(),
            updated_at: 1,
            read_only: false,
        }];
        let weak = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            window.resize(size(px(560.), px(480.)));
            window.dispatch_keystroke(Keystroke::parse("m").unwrap(), cx);
            window.on_next_frame(move |window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    assert_eq!(this.skill_search_input.read(cx).value().as_ref(), "m");
                    assert!(!this.skill_items.iter().any(|skill| skill.name == "other"));
                    let id = "claude:minimal-design".to_string();
                    let summary = skills::Summary {
                        id: id.clone(), source: "claude".into(), agent: "Claude".into(),
                        name: "minimal-design".into(), description: "Design minimally".into(),
                        revision: "hash".into(), updated_at: 1, read_only: false,
                    };
                    this.event(CoreEvent::SkillsListed { query: "m".into(), offset: 0, items: vec![summary.clone()], source_counts: BTreeMap::from([("claude".into(), 1)]) }, window, cx);
                    assert_eq!(this.skill_items.len(), 1);
                    assert!(this.sidebar_rows().contains(&SidebarRow::SkillSource("claude".into())));
                    assert!(!this.sidebar_rows().contains(&SidebarRow::SkillSource("gemini".into())));
                    this.select_skill(id.clone(), window, cx);
                    this.event(CoreEvent::SkillRead { id, skill: skills::Skill {
                        summary, content: "---\nname: minimal-design\ndescription: Design minimally\n---\n\nUse space.".into(), files: vec![],
                    } }, window, cx);
                    this.skill_content_input.update(cx, |input, cx| input.set_value("Changed", window, cx));
                    assert!(this.skill_is_dirty(cx));
                    this.click_sidebar_row(SidebarRow::Skills, window, cx);
                    assert!(this.skill_loaded.is_some() && this.skill_error.is_some());
                    this.discard_skill_changes(window, cx);
                    assert!(!this.skill_is_dirty(cx));
                    this.click_sidebar_row(SidebarRow::Skills, window, cx);
                    assert!(this.skill_loaded.is_none() && this.skill_selected.is_none());
                    assert_eq!(this.sidebar_cursor, Some(SidebarRow::Skills));
                    this.new_skill(window, cx);
                    this.skill_name_input.update(cx, |input, cx| input.set_value("new-skill", window, cx));
                    this.click_sidebar_row(SidebarRow::Skills, window, cx);
                    assert!(this.skill_new && this.skill_error.is_some());
                    this.discard_skill_changes(window, cx);
                    assert!(!this.skill_new);
                    eprintln!("tervy-skills-smoke PASS search_focus=true results=true source_visibility=true parent_navigation=true draft_guard=true discard=true narrow_window=true");
                    cx.quit();
                });
            });
        });
        cx.notify();
    }
}
