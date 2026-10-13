//! Filesystem-backed global agent skills. Callers choose a registered source,
//! never an arbitrary path. Skill contents remain owned by the agent CLI.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs,
    io::Write as _,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const MAX_TEXT: usize = 128 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub agent: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub id: String,
    pub source: String,
    pub agent: String,
    pub name: String,
    pub description: String,
    pub revision: String,
    pub updated_at: i64,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    #[serde(flatten)]
    pub summary: Summary,
    pub content: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub id: Option<String>,
    pub source: String,
    pub name: String,
    pub content: String,
    pub expected_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileWrite {
    pub id: String,
    pub path: String,
    pub content: String,
    pub expected_revision: Option<String>,
}

pub struct Catalog {
    home: PathBuf,
}

impl Catalog {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    pub fn current() -> Self {
        Self::new(PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
    }

    pub fn sources(&self) -> Vec<Source> {
        [
            ("claude", "Claude", ".claude/skills"),
            ("codex", "Codex", ".codex/skills"),
            (
                "shared",
                "Shared · Codex, Gemini, OpenCode",
                ".agents/skills",
            ),
            ("gemini", "Gemini CLI", ".gemini/skills"),
            ("opencode", "OpenCode", ".config/opencode/skills"),
        ]
        .into_iter()
        .map(|(id, agent, path)| Source {
            id: id.into(),
            agent: agent.into(),
            path: self.home.join(path),
        })
        .collect()
    }

    fn source(&self, id: &str) -> Result<Source, String> {
        self.sources()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| "Unknown skill source".into())
    }

    fn locate(&self, id: &str) -> Result<(Source, PathBuf, bool), String> {
        let (source_id, relative) = id.split_once(':').ok_or("Invalid skill ID")?;
        let source = self.source(source_id)?;
        let relative = Path::new(relative);
        if relative.components().count() == 0
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("Invalid skill ID".into());
        }
        let parts: Vec<_> = relative.components().collect();
        let managed = match source_id {
            "codex" => parts
                .first()
                .is_some_and(|part| part.as_os_str() == ".system"),
            "claude" => parts
                .first()
                .is_some_and(|part| part.as_os_str() == "synced"),
            _ => false,
        };
        if (!managed && parts.len() != 1)
            || (source_id == "codex" && managed && parts.len() != 2)
            || (source_id == "claude" && managed && parts.len() != 3)
        {
            return Err("Invalid skill ID".into());
        }
        let path = source.path.join(relative);
        Ok((source, path, managed))
    }

    pub fn list(&self, query: &str, limit: usize, offset: usize) -> Result<Vec<Summary>, String> {
        self.list_with_counts(query, limit, offset)
            .map(|(items, _)| items)
    }

    /// Counts include every matching skill, even when the requested page is full.
    pub fn list_with_counts(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<Summary>, BTreeMap<String, usize>), String> {
        let mut results = Vec::new();
        for source in self.sources() {
            self.scan(&source, &source.path, &mut results)?;
        }
        let query = query.trim().to_lowercase();
        results.retain(|s| {
            query.is_empty()
                || s.name.to_lowercase().contains(&query)
                || s.description.to_lowercase().contains(&query)
                || s.agent.to_lowercase().contains(&query)
        });
        let mut counts = BTreeMap::new();
        for skill in &results {
            *counts.entry(skill.source.clone()).or_insert(0) += 1;
        }
        results.sort_by(|a, b| {
            a.agent
                .cmp(&b.agent)
                .then(a.name.cmp(&b.name))
                .then(a.id.cmp(&b.id))
        });
        Ok((
            results
                .into_iter()
                .skip(offset.min(100_000))
                .take(limit.clamp(1, 100))
                .collect(),
            counts,
        ))
    }

    fn scan(&self, source: &Source, root: &Path, out: &mut Vec<Summary>) -> Result<(), String> {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("Cannot list {}: {error}", source.agent)),
        };
        for entry in entries.flatten().take(1000) {
            let path = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || name == "synced" {
                continue;
            }
            if entry.file_type().is_ok_and(|type_| type_.is_symlink()) {
                continue;
            }
            if !path.is_dir() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let Ok(relative) = path.strip_prefix(&source.path) else {
                continue;
            };
            let id = format!("{}:{}", source.id, relative.to_string_lossy());
            if let Ok(skill) = self.read(&id)
                && !skill.summary.read_only
            {
                out.push(skill.summary);
            }
        }
        Ok(())
    }

    pub fn read(&self, id: &str) -> Result<Skill, String> {
        let (source, directory, managed) = self.locate(id)?;
        let root = source
            .path
            .canonicalize()
            .map_err(|_| "Skill source not found")?;
        let actual = directory.canonicalize().map_err(|_| "Skill not found")?;
        if !actual.starts_with(&root) {
            return Err("Skill escapes its source".into());
        }
        let manifest = directory.join("SKILL.md");
        let metadata = fs::symlink_metadata(&manifest).map_err(|_| "SKILL.md not found")?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_TEXT as u64
        {
            return Err("SKILL.md is unavailable or too large".into());
        }
        let content = fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
        let (name, description) = metadata_fields(&content).unwrap_or_else(|error| {
            (
                directory
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                format!("Invalid SKILL.md: {error}"),
            )
        });
        let revision = digest(content.as_bytes());
        let updated_at = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|time| time.as_secs() as i64)
            .unwrap_or(0);
        let relative = directory
            .strip_prefix(&source.path)
            .map_err(|e| e.to_string())?;
        let linked =
            fs::symlink_metadata(&directory).is_ok_and(|meta| meta.file_type().is_symlink());
        let files = list_files(&directory, &directory, 0);
        Ok(Skill {
            summary: Summary {
                id: id.into(),
                source: source.id,
                agent: source.agent,
                name: if name.is_empty() {
                    relative
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into()
                } else {
                    name
                },
                description,
                revision,
                updated_at,
                read_only: managed || linked || !writable_directory(&directory),
            },
            content,
            files,
        })
    }

    pub fn write(&self, write: &Write) -> Result<Skill, String> {
        valid_name(&write.name)?;
        if write.content.len() > MAX_TEXT {
            return Err("Skill text exceeds 128 KiB".into());
        }
        let (name, description) = metadata_fields(&write.content)?;
        if name != write.name || description.trim().is_empty() {
            return Err("SKILL.md name must match its folder and have a description".into());
        }
        let source = self.source(&write.source)?;
        let id = write
            .id
            .clone()
            .unwrap_or_else(|| format!("{}:{}", source.id, write.name));
        let (actual_source, directory, managed) = self.locate(&id)?;
        if actual_source.id != source.id || managed {
            return Err("Skill is not editable".into());
        }
        if write.id.is_some() {
            let current = self.read(&id)?;
            if current.summary.read_only
                || current.summary.revision != write.expected_revision.clone().unwrap_or_default()
            {
                return Err("Skill changed; reload before saving".into());
            }
            if directory.file_name().unwrap_or_default() != write.name.as_str() {
                return Err("Skill folder name cannot change".into());
            }
        } else {
            if write.expected_revision.is_some() {
                return Err("New skill cannot have a revision".into());
            }
            fs::create_dir_all(&source.path).map_err(|e| e.to_string())?;
            fs::create_dir(&directory)
                .map_err(|_| "Skill already exists or cannot be created".to_string())?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        atomic_write(&directory.join("SKILL.md"), write.content.as_bytes())?;
        self.read(&id)
    }

    pub fn delete(&self, id: &str, revision: &str) -> Result<(), String> {
        let skill = self.read(id)?;
        if skill.summary.read_only || skill.summary.revision != revision {
            return Err("Skill changed or is read-only".into());
        }
        let (_, directory, _) = self.locate(id)?;
        fs::remove_dir_all(directory).map_err(|e| e.to_string())
    }

    pub fn read_file(&self, id: &str, relative: &str) -> Result<(String, String), String> {
        let skill = self.read(id)?;
        let (_, directory, _) = self.locate(&skill.summary.id)?;
        let path = safe_file(&directory, relative)?;
        let meta = fs::symlink_metadata(&path).map_err(|_| "Skill file not found")?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_TEXT as u64 {
            return Err("Skill file is unavailable or too large".into());
        }
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let revision = digest(content.as_bytes());
        Ok((content, revision))
    }

    pub fn write_file(&self, write: &FileWrite) -> Result<String, String> {
        let skill = self.read(&write.id)?;
        if skill.summary.read_only {
            return Err("Skill is read-only".into());
        }
        if write.content.len() > MAX_TEXT {
            return Err("Skill file exceeds 128 KiB".into());
        }
        let (_, directory, _) = self.locate(&write.id)?;
        let path = safe_file(&directory, &write.path)?;
        if path.file_name().is_some_and(|name| name == "SKILL.md") {
            return Err("Use update_skill for SKILL.md".into());
        }
        if path.exists() {
            let (_, revision) = self.read_file(&write.id, &write.path)?;
            if write.expected_revision.as_deref() != Some(&revision) {
                return Err("Skill file changed; reload before saving".into());
            }
        } else if write.expected_revision.is_some() {
            return Err("Skill file no longer exists".into());
        }
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        atomic_write(&path, write.content.as_bytes())?;
        Ok(digest(write.content.as_bytes()))
    }
}

fn writable_directory(path: &Path) -> bool {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(
            "Skill name must be lowercase letters, digits, and single hyphens (1–64 characters)"
                .into(),
        );
    }
    Ok(())
}

fn metadata_fields(content: &str) -> Result<(String, String), String> {
    let mut lines = content.lines();
    if lines.next() != Some("---") {
        return Err("SKILL.md needs YAML frontmatter".into());
    }
    let mut name = String::new();
    let mut description = String::new();
    let mut current = "";
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = value.trim().trim_matches(['\'', '"']).to_string();
            current = "name";
        } else if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            description = if matches!(value, "|" | "|-" | "|+" | ">" | ">-" | ">+") {
                String::new()
            } else {
                value.trim_matches(['\'', '"']).to_string()
            };
            current = "description";
        } else if (line.starts_with(' ') || line.starts_with('\t')) && current == "description" {
            if !description.is_empty() {
                description.push(' ');
            }
            description.push_str(line.trim());
        } else if !line.trim().is_empty() {
            current = "";
        }
    }
    if !closed || name.is_empty() || description.is_empty() {
        return Err("SKILL.md needs name and description in YAML frontmatter".into());
    }
    Ok((name, description))
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn safe_file(directory: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.components().count() == 0
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Invalid skill file path".into());
    }
    let mut path = directory.to_path_buf();
    for component in relative.components() {
        path.push(component.as_os_str());
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
        {
            return Err("Linked skill files cannot be edited through Tervy".into());
        }
    }
    Ok(path)
}

fn list_files(root: &Path, directory: &Path, depth: usize) -> Vec<String> {
    if depth > 3 {
        return vec![];
    }
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten().take(200) {
            let path = entry.path();
            if entry.file_type().is_ok_and(|type_| type_.is_symlink()) {
                continue;
            }
            if path.is_dir() {
                files.extend(list_files(root, &path, depth + 1));
            } else if path.is_file()
                && path.file_name().is_some_and(|name| name != "SKILL.md")
                && let Ok(relative) = path.strip_prefix(root)
            {
                files.push(relative.to_string_lossy().into());
            }
        }
    }
    files.sort();
    files
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid skill path")?;
    let temporary = parent.join(format!(".tervy-{}.tmp", uuid::Uuid::new_v4()));
    let permissions = fs::metadata(path)
        .map(|meta| meta.permissions())
        .unwrap_or_else(|_| fs::Permissions::from_mode(0o600));
    let result = (|| -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.set_permissions(permissions)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_conflicts() {
        let home = std::env::temp_dir().join(format!("tervy-skills-{}", uuid::Uuid::new_v4()));
        let catalog = Catalog::new(home.clone());
        let content = "---\nname: review-code\ndescription: Review code.\n---\n\nCheck it.\n";
        let skill = catalog
            .write(&Write {
                id: None,
                source: "claude".into(),
                name: "review-code".into(),
                content: content.into(),
                expected_revision: None,
            })
            .unwrap();
        assert_eq!(catalog.list("review", 100, 0).unwrap().len(), 1);
        assert_eq!(catalog.read(&skill.summary.id).unwrap().content, content);
        assert!(
            catalog
                .write(&Write {
                    id: Some(skill.summary.id.clone()),
                    source: "claude".into(),
                    name: "review-code".into(),
                    content: content.into(),
                    expected_revision: Some("stale".into())
                })
                .is_err()
        );
        let file = FileWrite {
            id: skill.summary.id.clone(),
            path: "references/checklist.md".into(),
            content: "hello".into(),
            expected_revision: None,
        };
        assert!(catalog.write_file(&file).is_ok());
        assert!(catalog.write_file(&file).is_err());
        assert!(catalog.read_file(&skill.summary.id, "../outside").is_err());
        catalog
            .delete(&skill.summary.id, &skill.summary.revision)
            .unwrap();
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn managed_skills_are_hidden_from_lists_and_cannot_be_changed() {
        let home =
            std::env::temp_dir().join(format!("tervy-managed-skills-{}", uuid::Uuid::new_v4()));
        let catalog = Catalog::new(home.clone());
        let managed = home.join(".codex/skills/.system/built-in");
        fs::create_dir_all(&managed).unwrap();
        fs::write(
            managed.join("SKILL.md"),
            "---\nname: built-in\ndescription: System skill\n---\n",
        )
        .unwrap();
        let skill = catalog.read("codex:.system/built-in").unwrap();
        assert!(skill.summary.read_only);
        assert!(catalog.list("built-in", 10, 0).unwrap().is_empty());
        assert!(catalog.list_with_counts("", 10, 0).unwrap().1.is_empty());
        assert!(
            catalog
                .write(&Write {
                    id: Some(skill.summary.id.clone()),
                    source: "codex".into(),
                    name: "built-in".into(),
                    content: skill.content.clone(),
                    expected_revision: Some(skill.summary.revision.clone()),
                })
                .is_err()
        );
        assert!(
            catalog
                .delete(&skill.summary.id, &skill.summary.revision)
                .is_err()
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn unwritable_skill_is_hidden_from_lists() {
        let home =
            std::env::temp_dir().join(format!("tervy-locked-skill-{}", uuid::Uuid::new_v4()));
        let catalog = Catalog::new(home.clone());
        let skill = catalog
            .write(&Write {
                id: None,
                source: "claude".into(),
                name: "locked".into(),
                content: "---\nname: locked\ndescription: Test an unwritable folder.\n---\n".into(),
                expected_revision: None,
            })
            .unwrap();
        let directory = home.join(".claude/skills/locked");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(catalog.read(&skill.summary.id).unwrap().summary.read_only);
        assert!(catalog.list("", 10, 0).unwrap().is_empty());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn every_registered_global_source_can_create_a_skill() {
        let home = std::env::temp_dir().join(format!("tervy-sources-{}", uuid::Uuid::new_v4()));
        let catalog = Catalog::new(home.clone());
        for source in catalog.sources() {
            let skill = catalog.write(&Write {
                id: None,
                source: source.id.clone(),
                name: "sample-skill".into(),
                content: "---\nname: sample-skill\ndescription: Use for a sample task.\n---\n\nWork.\n".into(),
                expected_revision: None,
            }).unwrap();
            assert!(source.path.join("sample-skill/SKILL.md").is_file());
            assert_eq!(skill.summary.source, source.id);
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn source_counts_hide_empty_agents_and_include_skills_beyond_page() {
        let home =
            std::env::temp_dir().join(format!("tervy-source-counts-{}", uuid::Uuid::new_v4()));
        let catalog = Catalog::new(home.clone());
        fs::create_dir_all(home.join(".gemini/skills")).unwrap();
        for source in ["claude", "codex"] {
            catalog.write(&Write {
                id: None,
                source: source.into(),
                name: "sample".into(),
                content: "---\nname: sample\ndescription: For testing source visibility.\n---\n\nWork.\n".into(),
                expected_revision: None,
            }).unwrap();
        }
        let (first_page, counts) = catalog.list_with_counts("", 1, 0).unwrap();
        assert_eq!(first_page.len(), 1);
        assert_eq!(counts.get("claude"), Some(&1));
        assert_eq!(counts.get("codex"), Some(&1));
        assert!(!counts.contains_key("gemini"));
        let (_, matching) = catalog.list_with_counts("codex", 1, 0).unwrap();
        assert_eq!(matching.len(), 1);
        assert!(!matching.contains_key("claude"));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn linked_skill_outside_registered_root_is_not_exposed() {
        let home =
            std::env::temp_dir().join(format!("tervy-linked-skill-{}", uuid::Uuid::new_v4()));
        let outside = home.join("outside");
        let root = home.join(".claude/skills");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            outside.join("SKILL.md"),
            "---\nname: escape\ndescription: Outside\n---\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let catalog = Catalog::new(home.clone());
        assert!(catalog.read("claude:escape").is_err());
        assert!(catalog.list("escape", 10, 0).unwrap().is_empty());
        fs::remove_dir_all(home).unwrap();
    }
}
