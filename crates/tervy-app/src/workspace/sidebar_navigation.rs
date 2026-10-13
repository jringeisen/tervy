//! Keyboard navigation follows rendered rows, independent of terminal focus.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SidebarRow {
    Skills,
    SkillSource(String),
    Skill(String),
    Notes,
    Meetings,
    Note(String),
    Meeting(String),
    Project(String),
    Session(SessionId),
}

pub(crate) fn key_bindings() -> [KeyBinding; 7] {
    [
        KeyBinding::new("cmd-0", FocusSidebar, Some("Workspace")),
        KeyBinding::new("up", SidebarUp, Some("Sidebar")),
        KeyBinding::new("down", SidebarDown, Some("Sidebar")),
        KeyBinding::new("enter", SidebarActivate, Some("Sidebar")),
        KeyBinding::new("left", SidebarCollapse, Some("Sidebar")),
        KeyBinding::new("right", SidebarExpand, Some("Sidebar")),
        KeyBinding::new("escape", SidebarExit, Some("Sidebar")),
    ]
}

fn next_row(
    rows: &[SidebarRow],
    current: Option<&SidebarRow>,
    direction: i32,
) -> Option<SidebarRow> {
    let index = current.and_then(|row| rows.iter().position(|candidate| candidate == row));
    let next = match (index, direction < 0) {
        (Some(index), true) => index.saturating_sub(1),
        (Some(index), false) => (index + 1).min(rows.len().saturating_sub(1)),
        (None, true) => rows.len().saturating_sub(1),
        (None, false) => 0,
    };
    rows.get(next).cloned()
}

impl Workspace {
    pub(super) fn toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        if self.sidebar_collapsed && self.sidebar_focus.contains_focused(window, cx) {
            self.focus.focus(window, cx);
            self.restore_focus(window, cx);
        }
        self.save_layout();
        cx.notify();
    }

    pub(super) fn ordered_agent_ids(&self) -> Vec<SessionId> {
        let mut ids: Vec<_> = self
            .tabs
            .iter()
            .filter(|(_, tab)| tab.info.agent.is_some())
            .map(|(id, _)| *id)
            .collect();
        ids.sort_by_key(|id| {
            (
                self.side_parent(*id).unwrap_or(*id),
                is_side(&self.tabs[id].info),
                *id,
            )
        });
        ids
    }

    pub(super) fn sidebar_rows(&self) -> Vec<SidebarRow> {
        let mut rows = Vec::new();
        let agents = self.ordered_agent_ids();
        for project in &self.projects {
            rows.push(SidebarRow::Project(project.id.clone()));
            if !self.expanded.contains(&project.id) {
                continue;
            }
            rows.extend(
                self.tabs
                    .iter()
                    .filter(|(_, tab)| {
                        tab.info.project == project.id
                            && tab.info.process.is_some()
                            && tab.info.agent.is_none()
                    })
                    .map(|(id, _)| SidebarRow::Session(*id)),
            );
            rows.extend(
                self.tab_order
                    .iter()
                    .filter(|id| {
                        self.tabs.get(id).is_some_and(|tab| {
                            tab.info.project == project.id
                                && tab.info.process.is_none()
                                && tab.info.agent.is_none()
                                && tab.open
                        })
                    })
                    .map(|id| SidebarRow::Session(*id)),
            );
            rows.extend(
                agents
                    .iter()
                    .filter(|id| self.tabs[id].info.project == project.id)
                    .map(|id| SidebarRow::Session(*id)),
            );
        }
        rows.push(SidebarRow::Skills);
        if self.skills_expanded {
            for source in skills::Catalog::current().sources() {
                if self
                    .skill_source_counts
                    .get(&source.id)
                    .copied()
                    .unwrap_or(0)
                    == 0
                {
                    continue;
                }
                rows.push(SidebarRow::SkillSource(source.id.clone()));
                if self.skill_sources_expanded.contains(&source.id) {
                    rows.extend(
                        self.skill_items
                            .iter()
                            .filter(|skill| skill.source == source.id)
                            .map(|skill| SidebarRow::Skill(skill.id.clone())),
                    );
                }
            }
        }
        rows.push(SidebarRow::Notes);
        if self.notes_expanded {
            rows.extend(
                self.notes
                    .iter()
                    .map(|note| SidebarRow::Note(note.id.clone())),
            );
        }
        rows.push(SidebarRow::Meetings);
        if self.meetings_expanded {
            rows.extend(
                self.meetings
                    .iter()
                    .map(|meeting| SidebarRow::Meeting(meeting.id.clone())),
            );
        }
        rows
    }

    pub(super) fn sync_sidebar_cursor(&mut self) {
        let rows = self.sidebar_rows();
        if self
            .sidebar_cursor
            .as_ref()
            .is_some_and(|row| rows.contains(row))
        {
            return;
        }
        self.sidebar_cursor = self
            .skills_open
            .then(|| {
                self.skill_selected
                    .clone()
                    .map(SidebarRow::Skill)
                    .unwrap_or(SidebarRow::Skills)
            })
            .filter(|row| rows.contains(row))
            .or_else(|| {
                self.notes_open
                    .then(|| {
                        if self.notes_expanded {
                            self.note_selected
                                .clone()
                                .map(SidebarRow::Note)
                                .unwrap_or(SidebarRow::Notes)
                        } else {
                            SidebarRow::Notes
                        }
                    })
                    .filter(|row| rows.contains(row))
            })
            .or_else(|| {
                self.meetings_open
                    .then(|| {
                        if self.meetings_expanded {
                            self.meeting_selected
                                .clone()
                                .map(SidebarRow::Meeting)
                                .unwrap_or(SidebarRow::Meetings)
                        } else {
                            SidebarRow::Meetings
                        }
                    })
                    .filter(|row| rows.contains(row))
            })
            .or_else(|| {
                self.side_focus
                    .or_else(|| self.active())
                    .map(SidebarRow::Session)
                    .filter(|row| rows.contains(row))
            })
            .or_else(|| {
                self.active_project
                    .clone()
                    .map(SidebarRow::Project)
                    .filter(|row| rows.contains(row))
            })
            .or_else(|| rows.first().cloned());
    }

    pub(super) fn focus_sidebar(
        &mut self,
        _: &FocusSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.setup_open
            || self.palette_open
            || self.process_project.is_some()
            || self.rename.is_some()
            || self.question_parent.is_some()
            || self.customization_provider.is_some()
            || self.customization_preview.is_some()
            || self.agent_launch.is_some()
        {
            return;
        }
        if self.sidebar_collapsed {
            self.sidebar_collapsed = false;
            self.save_layout();
        }
        if !self.sidebar_focus.contains_focused(window, cx) {
            self.sidebar_cursor = None;
        }
        self.sync_sidebar_cursor();
        self.sidebar_focus.focus(window, cx);
        self.sidebar_scroll_pending = true;
        cx.notify();
    }

    pub(super) fn move_sidebar(
        &mut self,
        direction: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_sidebar_cursor();
        self.sidebar_cursor = next_row(
            &self.sidebar_rows(),
            self.sidebar_cursor.as_ref(),
            direction,
        );
        self.sidebar_scroll_pending = true;
        self.sidebar_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn click_sidebar_row(
        &mut self,
        row: SidebarRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus_search = matches!(row, SidebarRow::Skills | SidebarRow::Notes);
        let opened = match &row {
            SidebarRow::Skills => self.open_skills(window, cx),
            SidebarRow::SkillSource(source) => {
                if self.open_skills(window, cx) {
                    if !self.skill_sources_expanded.insert(source.clone()) {
                        self.skill_sources_expanded.remove(source);
                    }
                    self.save_layout();
                    true
                } else {
                    false
                }
            }
            SidebarRow::Skill(id) => {
                self.select_skill(id.clone(), window, cx);
                self.skills_open && self.skill_selected.as_ref() == Some(id)
            }
            SidebarRow::Notes => self.open_notes(window, cx),
            SidebarRow::Meetings => {
                self.open_meetings(window, cx);
                true
            }
            SidebarRow::Note(id) => {
                self.select_note(id.clone(), window, cx);
                self.notes_open && self.note_selected.as_ref() == Some(id)
            }
            SidebarRow::Meeting(id) => {
                self.open_meetings(window, cx);
                self.select_meeting(id.clone(), window, cx);
                true
            }
            SidebarRow::Project(project) => {
                self.open_project(project.clone(), window, cx);
                true
            }
            SidebarRow::Session(id) => {
                self.choose_tab(*id, window, cx);
                true
            }
        };
        if !opened {
            return;
        }
        self.sidebar_cursor = Some(row);
        self.sidebar_scroll_pending = true;
        if !focus_search {
            self.sidebar_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn activate_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_sidebar_cursor();
        // Leave Sidebar before activation so terminal focus can be restored.
        self.focus.focus(window, cx);
        match self.sidebar_cursor.clone() {
            Some(SidebarRow::Skills) => {
                self.open_skills(window, cx);
            }
            Some(SidebarRow::SkillSource(source)) => {
                if self.open_skills(window, cx)
                    && !self.skill_sources_expanded.insert(source.clone())
                {
                    self.skill_sources_expanded.remove(&source);
                }
                self.save_layout();
            }
            Some(SidebarRow::Skill(id)) => self.select_skill(id, window, cx),
            Some(SidebarRow::Notes) => {
                self.open_notes(window, cx);
            }
            Some(SidebarRow::Meetings) => self.open_meetings(window, cx),
            Some(SidebarRow::Note(id)) => self.select_note(id, window, cx),
            Some(SidebarRow::Meeting(id)) => {
                self.open_meetings(window, cx);
                self.select_meeting(id, window, cx);
            }
            Some(SidebarRow::Project(project)) => self.open_project(project, window, cx),
            Some(SidebarRow::Session(id)) => self.choose_tab(id, window, cx),
            None => self.restore_focus(window, cx),
        }
        cx.notify();
    }

    pub(super) fn expand_sidebar(
        &mut self,
        expand: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_sidebar_cursor();
        match self.sidebar_cursor.clone() {
            Some(SidebarRow::Skills) => {
                self.skills_expanded = expand;
                if expand {
                    self.refresh_skills(cx);
                }
                self.sidebar_scroll_pending = true;
                self.save_layout();
                cx.notify();
                return;
            }
            Some(SidebarRow::SkillSource(source)) => {
                if expand {
                    self.skill_sources_expanded.insert(source);
                } else {
                    self.skill_sources_expanded.remove(&source);
                }
                self.sidebar_scroll_pending = true;
                self.save_layout();
                cx.notify();
                return;
            }
            Some(SidebarRow::Skill(_)) if !expand => {
                self.sidebar_cursor = Some(SidebarRow::Skills);
                self.sidebar_scroll_pending = true;
                cx.notify();
                return;
            }
            Some(SidebarRow::Notes) => {
                self.notes_expanded = expand;
                if expand {
                    self.refresh_notes(cx);
                }
                self.sidebar_scroll_pending = true;
                self.save_layout();
                cx.notify();
                return;
            }
            Some(SidebarRow::Meetings) => {
                self.meetings_expanded = expand;
                if expand {
                    self.core.send(CoreCommand::ListMeetings);
                }
                self.sidebar_scroll_pending = true;
                self.save_layout();
                cx.notify();
                return;
            }
            Some(SidebarRow::Note(_)) if !expand => {
                self.sidebar_cursor = Some(SidebarRow::Notes);
                self.sidebar_scroll_pending = true;
                cx.notify();
                return;
            }
            Some(SidebarRow::Meeting(_)) if !expand => {
                self.sidebar_cursor = Some(SidebarRow::Meetings);
                self.sidebar_scroll_pending = true;
                cx.notify();
                return;
            }
            _ => {}
        }
        let project = match self.sidebar_cursor.clone() {
            Some(SidebarRow::Project(project)) => {
                if expand {
                    self.expanded.insert(project.clone());
                } else {
                    self.expanded.remove(&project);
                }
                project
            }
            Some(SidebarRow::Session(id)) if !expand => {
                let Some(tab) = self.tabs.get(&id) else {
                    return;
                };
                tab.info.project.clone()
            }
            _ => return,
        };
        self.sidebar_cursor = Some(SidebarRow::Project(project));
        self.sidebar_scroll_pending = true;
        self.sidebar_focus.focus(window, cx);
        self.save_layout();
        cx.notify();
    }

    pub(super) fn exit_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.restore_focus(window, cx);
        cx.notify();
    }

    pub(super) fn sidebar_indicator(
        &self,
        row: SidebarRow,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let highlighted = self.sidebar_focus.contains_focused(window, cx)
            && self.sidebar_cursor.as_ref() == Some(&row);
        let scroll = highlighted && self.sidebar_scroll_pending;
        let color = cx.theme().ring;
        let scroll_handle = self.sidebar_scroll.clone();
        canvas(
            move |bounds, window, _| {
                if scroll {
                    let viewport = scroll_handle.bounds();
                    let adjustment = if bounds.top() < viewport.top() {
                        viewport.top() - bounds.top()
                    } else if bounds.bottom() > viewport.bottom() {
                        viewport.bottom() - bounds.bottom()
                    } else {
                        px(0.)
                    };
                    if adjustment != px(0.) {
                        let mut offset = scroll_handle.offset();
                        offset.y += adjustment;
                        scroll_handle.set_offset(offset);
                        window.refresh();
                    }
                }
            },
            move |bounds, _, window, _| {
                if highlighted {
                    let mut quad = outline(bounds, color, BorderStyle::default());
                    quad.corner_radii = px(6.).into();
                    window.paint_quad(quad);
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{SidebarRow, key_bindings, next_row};
    use gpui_kit::{KeyContext, Keymap, Keystroke};
    use tervy_core::supervisor::SessionId;

    #[test]
    fn navigation_stops_at_edges_and_recovers_from_missing_rows() {
        let project = SidebarRow::Project("one".into());
        let session = SidebarRow::Session(SessionId(1));
        let other = SidebarRow::Project("two".into());
        let rows = [project.clone(), session.clone(), other.clone()];
        assert_eq!(next_row(&rows, Some(&project), -1), Some(project.clone()));
        assert_eq!(next_row(&rows, Some(&project), 1), Some(session.clone()));
        assert_eq!(next_row(&rows, Some(&session), 1), Some(other.clone()));
        assert_eq!(next_row(&rows, Some(&other), 1), Some(other.clone()));
        let collapsed = [project.clone(), other.clone()];
        assert_eq!(next_row(&collapsed, Some(&project), 1), Some(other.clone()));
        assert_eq!(next_row(&collapsed, Some(&session), 1), Some(project));
        assert_eq!(next_row(&collapsed, None, -1), Some(other));
        assert_eq!(next_row(&[], None, 1), None);
    }

    #[test]
    fn arrow_bindings_are_confined_to_sidebar_context() {
        let mut map = Keymap::default();
        map.add_bindings(key_bindings());
        for key in ["up", "down", "enter", "left", "right", "escape"] {
            for context in ["Sidebar", "Terminal", "Input", "Command"] {
                let (bindings, pending) = map.bindings_for_input(
                    &[Keystroke::parse(key).unwrap()],
                    &[
                        KeyContext::parse("Workspace").unwrap(),
                        KeyContext::parse(context).unwrap(),
                    ],
                );
                assert_eq!(
                    bindings.len(),
                    usize::from(context == "Sidebar"),
                    "{key} in {context}"
                );
                assert!(!pending);
            }
        }
    }
}
