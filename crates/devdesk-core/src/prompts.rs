//! Shared prompt records. The supervisor owns all persistence and revisions.
use serde::{Deserialize, Serialize};

pub const MAX_TITLE: usize = 160;
pub const MAX_BODY: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    pub id: String,
    pub title: String,
    pub body: String,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Write {
    pub id: Option<String>,
    pub title: String,
    pub body: String,
    pub expected_revision: Option<i64>,
}

impl Write {
    pub fn validate(&self) -> Result<(), String> {
        let title = self.title.trim();
        if title.is_empty() || title.len() > MAX_TITLE || title.contains(['\n', '\r', '\0']) {
            return Err("Prompt title must be one line, 1–160 bytes".into());
        }
        if self.body.trim().is_empty() || self.body.len() > MAX_BODY || self.body.contains('\0') {
            return Err("Prompt body must contain text and be at most 64 KiB".into());
        }
        match (&self.id, self.expected_revision) {
            (None, None) => Ok(()),
            (Some(id), Some(revision)) if uuid::Uuid::parse_str(id).is_ok() && revision > 0 => {
                Ok(())
            }
            _ => Err("Updates require a prompt ID and current revision".into()),
        }
    }
}

pub fn validate_delete(id: &str, revision: i64) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_err() || revision < 1 {
        return Err("Delete requires a prompt ID and current revision".into());
    }
    Ok(())
}
