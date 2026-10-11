//! Codex config is read on a spawn worker. Only notification argv and MCP names
//! are retained; parse errors never echo config text, which may contain secrets.
use crate::{
    agent::{AgentProvider, Context, LaunchProfile, OBSERVER_PREAMBLE, Request},
    env::ShellEnvironment,
};
use devdesk_term::SpawnSpec;
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};

pub const DISABLED_SIDE_FEATURES: &[&str] = &[
    "plugins",
    "apps",
    "computer_use",
    "browser_use",
    "browser_use_external",
    "in_app_browser",
    "image_generation",
    "hooks",
    "skill_mcp_dependency_install",
    "tool_suggest",
    "worktrees",
    "workspace_dependencies",
];
#[derive(Default)]
pub struct Settings {
    pub servers: BTreeSet<String>,
    pub notify: Vec<String>,
}
impl Settings {
    pub fn read(environment: &ShellEnvironment, cwd: &Path, side: bool) -> Result<Self, String> {
        let home = environment
            .variables
            .get("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                environment
                    .variables
                    .get("HOME")
                    .map(|home| PathBuf::from(home).join(".codex"))
            })
            .ok_or("Cannot locate Codex config without HOME or CODEX_HOME")?;
        let user = read(&home.join("config.toml"), side)?;
        let mut settings = Self::default();
        if side && let Some(system) = read(Path::new("/etc/codex/config.toml"), false)? {
            settings.servers_from(&system)?;
        }
        if let Some(user) = &user {
            settings.servers_from(user)?;
            settings.notify_from(user)?;
            if let Some(profile) = user.get("profile") {
                let profile = profile.as_str().ok_or("Codex profile must be a string")?;
                if profile.is_empty()
                    || !profile
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    return Err("Codex profile name is not a safe profile filename".into());
                }
                let profile = read(&home.join(format!("{profile}.config.toml")), true)?.unwrap();
                settings.servers_from(&profile)?;
                settings.notify_from(&profile)?;
            }
        }
        // Include every layer from cwd to its Git root, whether trusted or not:
        // disabling an ignored server is harmless; missing a loaded one is not.
        for directory in cwd.ancestors().filter(|_| side) {
            if let Some(project) = read(&directory.join(".codex/config.toml"), false)? {
                settings.servers_from(&project)?;
            }
            if directory.join(".git").exists() {
                break;
            }
        }
        Ok(settings)
    }
    fn servers_from(&mut self, value: &toml::Value) -> Result<(), String> {
        if let Some(servers) = value.get("mcp_servers") {
            let servers = servers
                .as_table()
                .ok_or("Codex mcp_servers must be a table")?;
            if servers.len() > 512 {
                return Err("Codex config has too many MCP servers".into());
            }
            for (name, server) in servers {
                if !server.is_table() {
                    return Err("Codex MCP server definition must be a table".into());
                }
                if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                    return Err("Codex MCP server name is invalid".into());
                }
                self.servers.insert(name.clone());
                if self.servers.len() > 512 {
                    return Err("Codex config has too many MCP servers across layers".into());
                }
            }
        }
        Ok(())
    }
    fn notify_from(&mut self, value: &toml::Value) -> Result<(), String> {
        if let Some(notify) = value.get("notify") {
            let notify = notify
                .as_array()
                .ok_or("Codex notify must be a string array")?;
            if notify.len() > 64 {
                return Err("Codex notify has too many arguments".into());
            }
            self.notify = notify
                .iter()
                .map(|arg| {
                    arg.as_str()
                        .filter(|s| s.len() <= 4096 && !s.contains('\0'))
                        .map(str::to_owned)
                        .ok_or("Codex notify argument is invalid".into())
                })
                .collect::<Result<_, String>>()?;
            if self
                .notify
                .first()
                .is_some_and(|program| program.is_empty())
            {
                return Err("Codex notify program is empty".into());
            }
        }
        Ok(())
    }
}
fn read(path: &Path, required: bool) -> Result<Option<toml::Value>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(format!(
                "Cannot read Codex config {}; refusing an unchecked launch",
                path.display()
            ));
        }
    };
    let mut text = String::new();
    file.take(1_048_577)
        .read_to_string(&mut text)
        .map_err(|_| format!("Cannot read Codex config {}", path.display()))?;
    if text.len() > 1_048_576 {
        return Err("Codex config exceeds 1 MiB".into());
    }
    toml::from_str(&text).map(Some).map_err(|_| {
        format!(
            "Invalid TOML in Codex config {}; refusing an unchecked launch",
            path.display()
        )
    })
}

pub struct CodexProvider;
impl AgentProvider for CodexProvider {
    fn check_installed(&self, environment: &ShellEnvironment) -> Result<PathBuf, String> {
        crate::agent::installed("codex", environment)
    }
    fn build_launch(
        &self,
        c: &Context<'_>,
        environment: &ShellEnvironment,
    ) -> Result<SpawnSpec, String> {
        let side = c.record.role.profile() == LaunchProfile::ReadOnlySide;
        if side && !matches!(c.request, Request::Fork { .. } | Request::Resume) {
            return Err("A side conversation needs a fork or its own saved ID".into());
        }
        if !side && matches!(c.request, Request::Fork { .. }) {
            return Err("A fork must persist the read-only Side role".into());
        }
        let mut args = match c.request {
            Request::New | Request::Prompt(_) => vec![],
            Request::Resume => vec![
                "resume".into(),
                c.record
                    .session_id
                    .clone()
                    .ok_or("Codex has not reported a thread ID; use its resume picker")?,
            ],
            Request::ResumePicker => vec!["resume".into()],
            Request::Fork {
                parent_session_id, ..
            } => vec!["fork".into(), parent_session_id.clone()],
        };
        args.extend(crate::agent::launch_args(c.record)?);
        args.push("--no-daemon".into());
        let settings = Settings::read(environment, &c.record.cwd, side)?;
        if settings.servers.contains("tervy_local") {
            return Err(
                "Codex already has an MCP server named tervy_local; rename it to use Tervy prompts"
                    .into(),
            );
        }
        let helper = vec![
            c.helper.to_string_lossy().into_owned(),
            "hook".into(),
            "codex-notify".into(),
        ];
        args.extend([
            "-c".into(),
            format!(
                "notify={}",
                serde_json::to_string(&helper).map_err(|e| e.to_string())?
            ),
            "-c".into(),
            "tui.notifications=[\"approval-requested\"]".into(),
            "-c".into(),
            "tui.notification_method=\"bel\"".into(),
            "-c".into(),
            "tui.notification_condition=\"always\"".into(),
            "-c".into(),
            "tui.terminal_title=[\"session-id\"]".into(),
        ]);
        args.extend([
            "-c".into(), format!("mcp_servers.tervy_local.command={}", serde_json::to_string(&c.helper.to_string_lossy()).map_err(|e| e.to_string())?),
            "-c".into(), if side { "mcp_servers.tervy_local.args=[\"mcp\",\"--read-only\"]" } else { "mcp_servers.tervy_local.args=[\"mcp\"]" }.into(),
            "-c".into(), "mcp_servers.tervy_local.env_vars=[\"DEVDESK_AGENT_ID\",\"DEVDESK_LAUNCH_ID\",\"DEVDESK_HOOK_TOKEN\",\"DEVDESK_MCP_SOCKET\",\"DEVDESK_MCP_ROLE\"]".into(),
            "-c".into(), "mcp_servers.tervy_local.enabled=true".into(),
        ]);
        if side {
            args.extend(["-s", "read-only", "-a", "never"].map(str::to_owned));
            for name in settings.servers {
                // Codex splits override paths on dots without parsing TOML keys.
                // Quoting a segment creates a different server, leaving the real
                // one enabled. Reject names that cannot be addressed exactly.
                if !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    return Err(
                        "Cannot safely disable a Codex MCP server with an unusual name".into(),
                    );
                }
                args.extend(["-c".into(), format!("mcp_servers.{name}.enabled=false")]);
            }
            for feature in DISABLED_SIDE_FEATURES {
                args.extend(["--disable".into(), (*feature).into()]);
            }
            args.extend(["-c".into(), "agents.enabled=false".into()]);
        }
        if let Request::Fork { question, .. } = c.request {
            args.push(format!("{OBSERVER_PREAMBLE}\n\nQuestion: {question}"));
        }
        if let Request::Prompt(prompt) = c.request {
            args.push(prompt.clone());
        }
        let mut variables = crate::agent::hook_environment(c, environment);
        // User hooks/notify execute outside the model sandbox. Primary preserves
        // notify; Side only reports status, so user callbacks cannot write files.
        variables.remove("DEVDESK_CODEX_NOTIFY");
        if !side && !settings.notify.is_empty() {
            variables.insert(
                "DEVDESK_CODEX_NOTIFY".into(),
                serde_json::to_string(&settings.notify).map_err(|e| e.to_string())?,
            );
        }
        Ok(SpawnSpec {
            program: self
                .check_installed(environment)?
                .to_string_lossy()
                .into_owned(),
            args,
            cwd: c.record.cwd.clone(),
            env: variables,
            scrollback_lines: c.lines,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Provider, Record, Role};
    use std::os::unix::fs::PermissionsExt;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Fixture, ShellEnvironment) {
        let folder = Fixture(
            std::env::temp_dir().join(format!("devdesk-codex-argv-{}", uuid::Uuid::new_v4())),
        );
        std::fs::create_dir_all(folder.0.join("nested/.codex")).unwrap();
        std::fs::create_dir_all(folder.0.join(".codex")).unwrap();
        std::fs::create_dir_all(folder.0.join(".git")).unwrap();
        let cli = folder.0.join("codex");
        std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let environment = ShellEnvironment {
            shell: "/bin/sh".into(),
            variables: [
                ("PATH".into(), folder.0.to_string_lossy().into_owned()),
                (
                    "CODEX_HOME".into(),
                    folder.0.join(".codex").to_string_lossy().into_owned(),
                ),
                (
                    "DEVDESK_CODEX_NOTIFY".into(),
                    "[\"unsafe inherited callback\"]".into(),
                ),
            ]
            .into(),
            warning: None,
        };
        (folder, environment)
    }
    fn record(folder: &Path) -> Record {
        Record {
            id: "10000000-0000-4000-8000-000000000001".into(),
            project: "p".into(),
            name: "Codex".into(),
            cwd: folder.join("nested"),
            role: Role::Primary,
            launch_args: vec![],
            provider: Provider::Codex,
            session_id: Some("20000000-0000-4000-8000-000000000002".into()),
            parent: None,
            snapshot_at: None,
        }
    }
    fn launch(
        record: &Record,
        request: &Request,
        environment: &ShellEnvironment,
    ) -> Result<SpawnSpec, String> {
        CodexProvider.build_launch(
            &Context {
                record,
                request,
                launch: 8,
                token: "test-token",
                socket: Path::new("/tmp/devdesk.sock"),
                helper: Path::new("/Applications/DevDesk.app/Contents/MacOS/devdesk"),
                lines: 5000,
            },
            environment,
        )
    }
    fn common() -> Vec<String> {
        ["--no-daemon", "-c", "notify=[\"/Applications/DevDesk.app/Contents/MacOS/devdesk\",\"hook\",\"codex-notify\"]", "-c", "tui.notifications=[\"approval-requested\"]", "-c", "tui.notification_method=\"bel\"", "-c", "tui.notification_condition=\"always\"", "-c", "tui.terminal_title=[\"session-id\"]", "-c", "mcp_servers.tervy_local.command=\"/Applications/DevDesk.app/Contents/MacOS/devdesk\"", "-c", "mcp_servers.tervy_local.args=[\"mcp\"]", "-c", "mcp_servers.tervy_local.env_vars=[\"DEVDESK_AGENT_ID\",\"DEVDESK_LAUNCH_ID\",\"DEVDESK_HOOK_TOKEN\",\"DEVDESK_MCP_SOCKET\",\"DEVDESK_MCP_ROLE\"]", "-c", "mcp_servers.tervy_local.enabled=true"].map(str::to_owned).into()
    }
    #[test]
    fn custom_flags_survive_resume_and_are_excluded_from_read_only_sides() {
        let (folder, environment) = fixture();
        std::fs::write(folder.0.join(".codex/config.toml"), "").unwrap();
        let mut record = record(&folder.0);
        record.launch_args = vec![
            "--approve-for-me".into(),
            "--model".into(),
            "value with spaces".into(),
        ];
        for request in [Request::New, Request::Resume, Request::ResumePicker] {
            let spec = launch(&record, &request, &environment).unwrap();
            assert!(
                spec.args
                    .windows(record.launch_args.len())
                    .any(|args| args == record.launch_args)
            );
            assert!(spec.args.contains(&"--no-daemon".into()));
        }
        record.role = Role::Side;
        let spec = launch(&record, &Request::Resume, &environment).unwrap();
        assert!(!spec.args.contains(&"--approve-for-me".into()));
        assert!(!spec.args.contains(&"value with spaces".into()));
        assert!(
            spec.args
                .windows(4)
                .any(|args| args == ["-s", "read-only", "-a", "never"])
        );
    }
    #[test]
    fn complete_codex_argv_profiles_notify_and_safe_mcp_keys() {
        let (folder, environment) = fixture();
        std::fs::write(folder.0.join(".codex/config.toml"), "notify=['/tmp/notify tool','argument with spaces']\n[mcp_servers.node_repl]\ncommand='fixture'\n[mcp_servers.herd]\ncommand='fixture'\n").unwrap();
        std::fs::write(
            folder.0.join("nested/.codex/config.toml"),
            "notify=['ignored project callback']\n[mcp_servers.laravel-boost]\ncommand='fixture'\n",
        )
        .unwrap();
        let mut record = record(&folder.0);
        for request in [Request::New, Request::Resume, Request::ResumePicker] {
            let spec = launch(&record, &request, &environment).unwrap();
            let mut expected = match request {
                Request::New => vec![],
                Request::Resume => vec!["resume".into(), record.session_id.clone().unwrap()],
                Request::ResumePicker => vec!["resume".into()],
                Request::Fork { .. } | Request::Prompt(_) => unreachable!(),
            };
            expected.extend(common());
            assert_eq!(spec.args, expected);
            assert_eq!(spec.cwd, record.cwd);
            assert_eq!(spec.scrollback_lines, 5000);
            assert_eq!(
                spec.env["DEVDESK_CODEX_NOTIFY"],
                "[\"/tmp/notify tool\",\"argument with spaces\"]"
            );
            assert_eq!(spec.env["DEVDESK_LAUNCH_ID"], "8");
        }
        let prompt = Request::Prompt("Make the font larger\nWrite proposal.toml".into());
        let spec = launch(&record, &prompt, &environment).unwrap();
        assert_eq!(
            spec.args.last().unwrap(),
            "Make the font larger\nWrite proposal.toml"
        );
        let mut side = record.clone();
        side.role = Role::Side;
        assert!(launch(&side, &prompt, &environment).is_err());
        record.role = Role::Side;
        let request = Request::Fork {
            parent_session_id: record.session_id.clone().unwrap(),
            question: "Why this choice?".into(),
        };
        let spec = launch(&record, &request, &environment).unwrap();
        let mut expected = vec!["fork".into(), record.session_id.clone().unwrap()];
        expected.extend(common());
        *expected
            .iter_mut()
            .find(|arg| arg.as_str() == "mcp_servers.tervy_local.args=[\"mcp\"]")
            .unwrap() = "mcp_servers.tervy_local.args=[\"mcp\",\"--read-only\"]".into();
        expected.extend(
            [
                "-s",
                "read-only",
                "-a",
                "never",
                "-c",
                "mcp_servers.herd.enabled=false",
                "-c",
                "mcp_servers.laravel-boost.enabled=false",
                "-c",
                "mcp_servers.node_repl.enabled=false",
                "--disable",
                "plugins",
                "--disable",
                "apps",
                "--disable",
                "computer_use",
                "--disable",
                "browser_use",
                "--disable",
                "browser_use_external",
                "--disable",
                "in_app_browser",
                "--disable",
                "image_generation",
                "--disable",
                "hooks",
                "--disable",
                "skill_mcp_dependency_install",
                "--disable",
                "tool_suggest",
                "--disable",
                "worktrees",
                "--disable",
                "workspace_dependencies",
                "-c",
                "agents.enabled=false",
            ]
            .map(str::to_owned),
        );
        expected.push(format!("{OBSERVER_PREAMBLE}\n\nQuestion: Why this choice?"));
        assert_eq!(spec.args, expected);
        assert!(!spec.env.contains_key("DEVDESK_CODEX_NOTIFY"));
        let resumed = launch(&record, &Request::Resume, &environment).unwrap();
        assert_eq!(
            &resumed.args[..2],
            ["resume", record.session_id.as_deref().unwrap()]
        );
        assert_eq!(resumed.args[2..], spec.args[2..spec.args.len() - 1]);
        assert!(launch(&record, &Request::New, &environment).is_err());
        assert!(launch(&record, &Request::ResumePicker, &environment).is_err());
        record.role = Role::Primary;
        assert!(launch(&record, &request, &environment).is_err());
    }
    #[test]
    fn unreadable_invalid_or_oversized_config_fails_closed_without_echoing_secrets() {
        let (folder, environment) = fixture();
        let mut record = record(&folder.0);
        record.role = Role::Side;
        assert!(launch(&record, &Request::Resume, &environment).is_err());
        let path = folder.0.join(".codex/config.toml");
        std::fs::write(&path, "api_key='DO_NOT_LOG_THIS_SECRET'\n[broken\n").unwrap();
        let error = launch(&record, &Request::Resume, &environment)
            .err()
            .unwrap();
        assert!(!error.contains("DO_NOT_LOG_THIS_SECRET"));
        std::fs::write(&path, "notify='not an array'\n").unwrap();
        assert!(launch(&record, &Request::Resume, &environment).is_err());
        std::fs::write(&path, "mcp_servers='not a table'\n").unwrap();
        assert!(launch(&record, &Request::Resume, &environment).is_err());
        std::fs::write(&path, "mcp_servers.bad='not a server definition'\n").unwrap();
        assert!(launch(&record, &Request::Resume, &environment).is_err());
        std::fs::write(&path, "[mcp_servers.\"a.b\"]\ncommand=\"fixture\"\n").unwrap();
        assert!(launch(&record, &Request::Resume, &environment).is_err());
        std::fs::write(&path, "#".repeat(1_048_577)).unwrap();
        assert!(launch(&record, &Request::Resume, &environment).is_err());
    }
    #[test]
    fn profile_and_ancestor_layers_are_enumerated_but_project_notify_is_ignored() {
        let (folder, environment) = fixture();
        let home = folder.0.join(".codex");
        std::fs::write(
            home.join("config.toml"),
            "profile='review'\nnotify=['base']\n[mcp_servers.base]\ncommand='fixture'\n",
        )
        .unwrap();
        std::fs::write(
            home.join("review.config.toml"),
            "notify=['profile']\n[mcp_servers.'quote\"name']\ncommand='fixture'\n",
        )
        .unwrap();
        std::fs::write(
            folder.0.join("nested/.codex/config.toml"),
            "notify=['project ignored']\n[mcp_servers.nested]\ncommand='fixture'\n",
        )
        .unwrap();
        let settings = Settings::read(&environment, &folder.0.join("nested"), true).unwrap();
        assert_eq!(
            settings.servers,
            ["base".into(), "nested".into(), "quote\"name".into()].into()
        );
        assert_eq!(settings.notify, ["profile"]);
        std::fs::remove_file(home.join("review.config.toml")).unwrap();
        assert!(Settings::read(&environment, &folder.0.join("nested"), true).is_err());
    }
}
