//! Serializable layout contains identities, never terminal output or hook tokens.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub active_project: Option<String>,
    pub tabs: Vec<Tab>,
    pub selected: Vec<String>,
    pub expanded: Vec<String>,
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
