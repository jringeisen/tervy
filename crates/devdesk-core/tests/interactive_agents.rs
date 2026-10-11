//! Exercise plain agent launch, stop, restart, and restore without model calls.
use devdesk_core::{
    agent::{Provider, Status},
    store::Store,
    supervisor::{Command, CoreEvent, SessionStatus, Supervisor},
};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};

fn event(runtime: &Supervisor, deadline: Instant) -> CoreEvent {
    loop {
        if let Ok(event) = runtime.events.try_recv() {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "interactive agent fixture timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn shutdown(runtime: &Supervisor, deadline: Instant) {
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } = event(runtime, deadline) {
            assert!(clean);
            return;
        }
    }
}

#[test]
fn interactive_agents_launch_stop_restart_and_restore() {
    // Each test subprocess gets a private shell PATH before the one-shot shell
    // environment initializes; no process-global environment mutation is needed.
    if let Some(folder) = std::env::var_os("DEVDESK_INTERACTIVE_AGENT_FIXTURE") {
        let folder = Path::new(&folder);
        let config = folder.join("config.toml");
        let runtime = Supervisor::start(config.clone(), None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        for provider in [Provider::Gemini, Provider::OpenCode, Provider::Aider] {
            runtime.handle.send(Command::NewAgentWithArgs {
                project: "p".into(),
                picker: false,
                provider,
                args: vec!["--custom-flag".into(), "value with spaces".into()],
            });
            let id = loop {
                match event(&runtime, deadline) {
                    CoreEvent::SessionStarted { info, .. }
                        if info
                            .agent
                            .as_ref()
                            .is_some_and(|agent| agent.record.provider == provider) =>
                    {
                        let agent = info.agent.unwrap();
                        assert_eq!(agent.status, Status::Running);
                        assert_eq!(
                            agent.record.launch_args,
                            ["--custom-flag", "value with spaces"]
                        );
                        break info.id;
                    }
                    CoreEvent::ConfigError(error) => panic!("{error}"),
                    _ => {}
                }
            };
            for restart in [true, false] {
                runtime.handle.send(Command::Stop(id));
                loop {
                    if let CoreEvent::SessionChanged(info) = event(&runtime, deadline)
                        && info.id == id
                        && matches!(
                            info.status,
                            SessionStatus::Stopped | SessionStatus::Exited { .. }
                        )
                    {
                        assert_eq!(info.agent.unwrap().status, Status::Exited);
                        break;
                    }
                }
                if restart {
                    runtime.handle.send(Command::ResumeAgentWithArgs(
                        id,
                        vec!["--updated-flag".into()],
                    ));
                    loop {
                        match event(&runtime, deadline) {
                            CoreEvent::SessionStarted { info, .. } if info.id == id => {
                                let agent = info.agent.unwrap();
                                assert_eq!(agent.status, Status::Running);
                                assert_eq!(agent.record.launch_args, ["--updated-flag"]);
                                break;
                            }
                            CoreEvent::ConfigError(error) => panic!("{error}"),
                            _ => {}
                        }
                    }
                }
            }
        }
        shutdown(&runtime, deadline);
        drop(runtime);
        let store = Store::open(&folder.join("state.db")).unwrap();
        let agents = store.agents().unwrap();
        assert_eq!(agents.len(), 3);
        assert!(
            agents
                .iter()
                .all(|agent| agent.launch_args == ["--updated-flag"])
        );
        assert!(store.survivors().unwrap().is_empty());
        drop(store);
        let runtime = Supervisor::start(config, None).unwrap();
        runtime.handle.send(Command::Restore);
        let mut restored = vec![];
        loop {
            match event(&runtime, deadline) {
                CoreEvent::SessionChanged(info) => {
                    if let Some(agent) = info.agent {
                        assert_eq!(agent.status, Status::Exited);
                        restored.push(agent.record.provider);
                    }
                }
                CoreEvent::SessionStarted { .. } => panic!("restore must not start agents"),
                CoreEvent::Restored { .. } => break,
                _ => {}
            }
        }
        for provider in [Provider::Gemini, Provider::OpenCode, Provider::Aider] {
            assert!(restored.contains(&provider));
        }
        shutdown(&runtime, deadline);
        return;
    }
    let folder = std::env::temp_dir().join(format!(
        "devdesk-interactive-fixture-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let shell = folder.join("shell");
    std::fs::write(
        &shell,
        "#!/bin/sh\nprintf '\\0DEVDESK_ENV\\0'\n/usr/bin/env -0\n",
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
    for provider in [Provider::Gemini, Provider::OpenCode, Provider::Aider] {
        let cli = folder.join(provider.command());
        std::fs::write(&cli, "#!/bin/sh\nexec /bin/sleep 120\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(
        folder.join("config.toml"),
        format!(
            "[[projects]]\nid='p'\nname='P'\npath={:?}\n",
            folder.to_string_lossy()
        ),
    )
    .unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "interactive_agents_launch_stop_restart_and_restore",
            "--nocapture",
        ])
        .env("DEVDESK_INTERACTIVE_AGENT_FIXTURE", &folder)
        .env("SHELL", shell)
        .env("PATH", format!("{}:/usr/bin:/bin", folder.display()))
        .status()
        .unwrap();
    std::fs::remove_dir_all(folder).unwrap();
    assert!(status.success());
}
