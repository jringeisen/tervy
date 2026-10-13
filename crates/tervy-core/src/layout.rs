//! Serializable layout contains identities, never terminal output or hook tokens.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// Logical pixels; optional for layouts written before sidebar resizing.
    #[serde(default)]
    pub sidebar_width: Option<f32>,
    #[serde(default)]
    pub sidebar_collapsed: bool,
    pub active_project: Option<String>,
    pub tabs: Vec<Tab>,
    pub selected: Vec<String>,
    pub expanded: Vec<String>,
    #[serde(default)]
    pub skills_expanded: bool,
    #[serde(default)]
    pub notes_expanded: bool,
    #[serde(default)]
    pub meetings_expanded: bool,
    #[serde(default)]
    pub skill_sources_expanded: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub key: String,
    pub project: String,
    pub name: String,
    pub kind: Kind,
    pub cwd: Option<PathBuf>,
    pub open: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Kind {
    Shell,
    Process(String),
    Agent(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_layouts_load_and_sidebar_width_round_trips() {
        let old = r#"{"active_project":null,"tabs":[],"selected":[],"expanded":[]}"#;
        let mut layout: Layout = serde_json::from_str(old).unwrap();
        assert_eq!(layout.sidebar_width, None);
        assert!(!layout.sidebar_collapsed);
        assert!(!layout.skills_expanded);
        assert!(!layout.notes_expanded);
        assert!(!layout.meetings_expanded);
        layout.sidebar_width = Some(340.);
        layout.sidebar_collapsed = true;
        layout.skills_expanded = true;
        layout.notes_expanded = true;
        layout.meetings_expanded = true;
        layout.skill_sources_expanded = vec!["codex".into()];
        let saved = serde_json::to_string(&layout).unwrap();
        assert_eq!(serde_json::from_str::<Layout>(&saved).unwrap(), layout);
    }
}
