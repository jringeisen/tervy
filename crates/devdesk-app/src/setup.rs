//! Read-only prerequisite checks. Never install tools or invoke an agent.
use devdesk_core::agent::Provider;
use devdesk_core::env::ShellEnvironment;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[derive(Clone, Debug)]
pub(crate) struct Readiness {
    pub shell: bool,
    pub agents: Vec<Provider>,
    pub meetings_supported: bool,
    pub warning: Option<String>,
}

impl Readiness {
    pub(crate) fn installed_agents(&self) -> Vec<Provider> {
        self.agents.clone()
    }
}

/// Keep onboarding separate from project configuration and restored sessions.
const WELCOME_MARKER: &str = "welcome-v1.seen";

pub(crate) fn welcome_needed(directory: &Path) -> bool {
    !directory.join(WELCOME_MARKER).is_file()
}

pub(crate) fn acknowledge_welcome(directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    std::fs::write(directory.join(WELCOME_MARKER), "Tervy welcome dismissed\n")
}

fn executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(crate) fn inspect() -> Readiness {
    let environment = ShellEnvironment::shared();
    Readiness {
        shell: executable(Path::new(&environment.shell)),
        agents: discover_agents(&environment),
        meetings_supported: devdesk_core::meetings::supported_os(),
        warning: environment.warning.clone(),
    }
}

fn discover_agents(environment: &ShellEnvironment) -> Vec<Provider> {
    Provider::ALL
        .into_iter()
        .filter(|provider| provider.check_installed(environment).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn welcome_is_remembered_across_launches_without_changing_config() {
        let dir = std::env::temp_dir().join(format!("tervy-welcome-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "existing project settings").unwrap();
        assert!(welcome_needed(&dir));
        acknowledge_welcome(&dir).unwrap();
        assert!(!welcome_needed(&dir));
        acknowledge_welcome(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("config.toml")).unwrap(),
            "existing project settings"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn readiness_requires_an_executable_file_on_the_resolved_path() {
        let dir = std::env::temp_dir().join(format!("devdesk-readiness-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("claude");
        let environment = ShellEnvironment {
            shell: "/bin/sh".into(),
            variables: [("PATH".into(), dir.to_string_lossy().into_owned())].into(),
            warning: None,
        };
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Provider::Claude.check_installed(&environment).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(Provider::Claude.check_installed(&environment).is_ok());
        assert!(Provider::Codex.check_installed(&environment).is_err());
        assert!(!executable(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn agent_choices_only_include_installed_providers() {
        let dir =
            std::env::temp_dir().join(format!("devdesk-agent-discovery-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let environment = ShellEnvironment {
            shell: "/bin/sh".into(),
            variables: [("PATH".into(), dir.to_string_lossy().into_owned())].into(),
            warning: None,
        };
        assert!(discover_agents(&environment).is_empty());
        let mut expected = vec![];
        for provider in Provider::ALL {
            let path = dir.join(provider.command());
            std::fs::write(&path, "#!/bin/sh\nexit 99\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            expected.push(provider);
            assert_eq!(discover_agents(&environment), expected);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
