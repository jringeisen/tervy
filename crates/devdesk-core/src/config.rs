//! Global, user-authored configuration. No repository config is read.
//! All filesystem work belongs to the supervisor thread, never a view update.
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub settings: Settings,
    pub projects: Vec<Project>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub scrollback_lines: usize,
    pub font_family: String,
    pub font_size: f32,
    pub option_as_meta: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            scrollback_lines: 5000,
            font_family: "Menlo".into(),
            font_size: 14.,
            option_as_meta: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub processes: Vec<ProcessSpec>,
    #[serde(default)]
    pub terminals: Vec<TerminalSpec>,
    #[serde(default)]
    pub agents: AgentDefaults,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSpec {
    pub id: String,
    pub name: String,
    pub command: String,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub restart: RestartPolicy,
    #[serde(default = "default_signal")]
    pub stop_signal: String,
    #[serde(default = "default_timeout")]
    pub stop_timeout_secs: u64,
    pub shell: Option<String>,
}
fn default_signal() -> String {
    "SIGINT".into()
}
fn default_timeout() -> u64 {
    10
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RestartPolicy {
    pub policy: String,
    pub max_retries: u32,
}
impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            policy: "never".into(),
            max_retries: 5,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalSpec {
    pub name: String,
    pub cwd: Option<PathBuf>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentDefaults {
    pub default_provider: Option<String>,
}

pub fn support_directory() -> PathBuf {
    std::env::var_os("DEVDESK_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Library/Application Support/DevDesk")
        })
}

pub fn parse(text: &str, home: &Path) -> Result<Config, String> {
    if text.len() > 1_000_000 {
        return Err("config.toml exceeds 1 MB".into());
    }
    let mut config: Config =
        toml::from_str(text).map_err(|error| format!("config.toml: {error}"))?;
    if config.settings.scrollback_lines > 100_000 {
        return Err("settings.scrollback_lines must be between 0 and 100000".into());
    }
    if !(8. ..=32.).contains(&config.settings.font_size)
        || config.settings.font_family.trim().is_empty()
    {
        return Err("font_size must be 8–32 and font_family must not be empty".into());
    }
    let mut ids = HashSet::new();
    for project in &mut config.projects {
        if !slug(&project.id) || !ids.insert(project.id.clone()) {
            return Err(format!(
                "Project ID '{}' must be unique and use letters, digits, '-' or '_'",
                project.id
            ));
        }
        if project.name.trim().is_empty() {
            return Err(format!("Project '{}' needs a name", project.id));
        }
        project.path = expand(&project.path, home);
        if !project.path.is_absolute() || !project.path.is_dir() {
            return Err(format!(
                "Project '{}': folder does not exist: {}",
                project.name,
                project.path.display()
            ));
        }
        let mut processes = HashSet::new();
        for process in &project.processes {
            if !slug(&process.id) || !processes.insert(&process.id) {
                return Err(format!(
                    "Project '{}': process ID '{}' is invalid or duplicated",
                    project.name, process.id
                ));
            }
            if process.command.trim().is_empty() || process.name.trim().is_empty() {
                return Err(format!(
                    "Process '{}': name and command are required",
                    process.id
                ));
            }
            if !["SIGINT", "SIGTERM", "SIGHUP"].contains(&process.stop_signal.as_str())
                || !(1..=300).contains(&process.stop_timeout_secs)
            {
                return Err(format!(
                    "Process '{}': invalid stop signal or timeout (1–300 seconds)",
                    process.id
                ));
            }
            if !["never", "on-failure"].contains(&process.restart.policy.as_str())
                || process.restart.max_retries > 100
            {
                return Err(format!("Process '{}': invalid restart policy", process.id));
            }
            if process
                .shell
                .as_deref()
                .is_some_and(|shell| shell != "interactive")
            {
                return Err(format!(
                    "Process '{}': shell may only be 'interactive'",
                    process.id
                ));
            }
            for key in process.env.keys() {
                if key.is_empty() || key.contains(['=', '\0']) {
                    return Err(format!("Process '{}': invalid environment key", process.id));
                }
            }
        }
        if project
            .agents
            .default_provider
            .as_deref()
            .is_some_and(|provider| !["claude", "codex"].contains(&provider))
        {
            return Err(format!(
                "Project '{}': unknown agent provider",
                project.name
            ));
        }
    }
    Ok(config)
}
fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
}
fn expand(path: &Path, home: &Path) -> PathBuf {
    if path == Path::new("~") {
        home.into()
    } else if let Ok(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else {
        path.into()
    }
}

pub fn read(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    parse(
        &text,
        &PathBuf::from(std::env::var_os("HOME").unwrap_or_default()),
    )
}

pub fn initialize(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("config path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) {
        Ok(mut file) => file.write_all(b"# DevDesk global configuration. Projects are added with Add Project.\n[settings]\nscrollback_lines = 5000\nfont_family = \"Menlo\"\nfont_size = 14\n").map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Read again at mutation time: never overwrite an invalid hand edit with
/// the actor's last-good mirror. toml_edit preserves existing comments.
pub fn append_project(path: &Path, folder: &Path) -> Result<(Config, String), String> {
    let current = read(path)?;
    let folder = std::fs::canonicalize(folder).map_err(|error| error.to_string())?;
    if !folder.is_dir() {
        return Err("Choose a folder".into());
    }
    if let Some(existing) = current
        .projects
        .iter()
        .find(|project| std::fs::canonicalize(&project.path).ok().as_ref() == Some(&folder))
    {
        return Ok((current.clone(), existing.id.clone()));
    }
    let name = folder
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let base: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let base = base.trim_matches('-');
    let base = if base.is_empty() { "project" } else { base };
    let mut id = base.chars().take(90).collect::<String>();
    let mut suffix = 2;
    while current.projects.iter().any(|project| project.id == id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut document: toml_edit::DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| error.to_string())?;
    if document.get("projects").is_none() {
        document["projects"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let projects = document["projects"]
        .as_array_of_tables_mut()
        .ok_or("projects must use [[projects]] tables")?;
    let mut project = toml_edit::Table::new();
    project["id"] = toml_edit::value(&id);
    project["name"] = toml_edit::value(name);
    project["path"] = toml_edit::value(folder.to_string_lossy().as_ref());
    projects.push(project);
    let output = document.to_string();
    let config = parse(
        &output,
        &PathBuf::from(std::env::var_os("HOME").unwrap_or_default()),
    )?;
    let temporary = path.with_file_name(format!(".config-{}.tmp", std::process::id()));
    let mut created = false;
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        created = true;
        file.write_all(output.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        // Catch hand edits made before this check. This is best-effort, not an
        // atomic compare-and-swap with external editors.
        if std::fs::read_to_string(path).map_err(|error| error.to_string())? != text {
            return Err("config.toml changed while adding the project; try again".into());
        }
        std::fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    if created && result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result?;
    Ok((config, id))
}

/// Remove only a DevDesk configuration entry; project files are never touched.
/// Re-read before atomic replacement to preserve comments and reject editor races.
pub fn remove(path: &Path, project_id: &str, process_id: Option<&str>) -> Result<Config, String> {
    let original = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut document: toml_edit::DocumentMut = original
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let projects = document
        .get_mut("projects")
        .and_then(|item| item.as_array_of_tables_mut())
        .ok_or("projects must use [[projects]] tables")?;
    let index = projects
        .iter()
        .position(|table| table.get("id").and_then(|item| item.as_str()) == Some(project_id))
        .ok_or("Project is no longer configured")?;
    if let Some(process_id) = process_id {
        let processes = projects
            .get_mut(index)
            .unwrap()
            .get_mut("processes")
            .and_then(|item| item.as_array_of_tables_mut())
            .ok_or("Process is no longer configured")?;
        let index = processes
            .iter()
            .position(|table| table.get("id").and_then(|item| item.as_str()) == Some(process_id))
            .ok_or("Process is no longer configured")?;
        processes.remove(index);
    } else {
        projects.remove(index);
    }
    let output = document.to_string();
    let config = parse(
        &output,
        &PathBuf::from(std::env::var_os("HOME").unwrap_or_default()),
    )?;
    let temporary = path.with_file_name(format!(".remove-{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(output.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        if std::fs::read_to_string(path).map_err(|e| e.to_string())? != original {
            return Err("config.toml changed while removing the entry; try again".into());
        }
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_duplicates_and_preserves_project_comments() {
        let directory = std::env::temp_dir().join(format!("devdesk-config-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        std::fs::write(
            &path,
            "# keep my comment\n[settings]\nscrollback_lines = 321\n",
        )
        .unwrap();
        let (config, id) = append_project(&path, &directory).unwrap();
        assert_eq!(config.settings.scrollback_lines, 321);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("# keep my comment")
        );
        assert_eq!(append_project(&path, &directory).unwrap().1, id);
        let duplicate = format!(
            "[[projects]]\nid='a'\nname='A'\npath='{}'\n",
            directory.display()
        );
        assert!(
            parse(&duplicate.repeat(2), &directory)
                .unwrap_err()
                .contains("unique")
        );
        assert!(parse("[settings]\nscrollback_lines=100001", &directory).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
