use std::time::{Duration, Instant};
use tervy_core::{
    config,
    supervisor::{Command, CoreEvent, SessionStatus, Supervisor},
};

#[test]
fn addition_preserves_comments_generates_unique_ids_and_rejects_invalid_edits() {
    let directory =
        std::env::temp_dir().join(format!("tervy-add-process-config-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("config.toml");
    let original = format!(
        "# my settings\n[settings]\nfont_size = 16 # keep this\n[[projects]]\nid='first'\nname='First'\npath='{}'\n[[projects]]\nid='second'\nname='Second'\npath='{}'\n",
        directory.display(),
        directory.display()
    );
    std::fs::write(&path, &original).unwrap();
    let command = "printf 'hello \"world\"' && npm run dev";
    config::append_process(&path, "second", " Vite Dev ", command).unwrap();
    let config = config::append_process(&path, "second", "Vite Dev", "npm run dev").unwrap();
    assert!(config.projects[0].processes.is_empty());
    let processes = &config.projects[1].processes;
    assert_eq!(processes[0].id, "vite-dev");
    assert_eq!(processes[1].id, "vite-dev-2");
    assert_eq!(processes[0].command, command);
    assert!(!processes[0].autostart);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("# my settings"));
    assert!(saved.contains("# keep this"));
    for (project, name, command) in [
        ("second", " ", "npm run dev"),
        ("second", "Vite", " "),
        ("missing", "Vite", "npm run dev"),
    ] {
        assert!(config::append_process(&path, project, name, command).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
    }
    std::fs::write(&path, "[settings]\nunknown=true\n").unwrap();
    assert!(config::append_process(&path, "second", "Vite", "npm run dev").is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[settings]\nunknown=true\n"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn supervisor_adds_a_stopped_process_without_executing_its_command() {
    let directory =
        std::env::temp_dir().join(format!("tervy-add-process-actor-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("config.toml");
    std::fs::write(
        &path,
        format!(
            "[[projects]]\nid='test'\nname='Test'\npath='{}'\n",
            directory.display()
        ),
    )
    .unwrap();
    let runtime = Supervisor::start(path.clone(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let next = || loop {
        if let Ok(event) = runtime.events.try_recv() {
            break event;
        }
        assert!(Instant::now() < deadline, "add process timed out");
        std::thread::sleep(Duration::from_millis(2));
    };
    while !matches!(next(), CoreEvent::Config(_)) {}
    runtime.handle.send(Command::AddProcess {
        project: "test".into(),
        name: "Marker".into(),
        command: format!("touch '{}'", directory.join("executed").display()),
    });
    let mut stopped = false;
    loop {
        match next() {
            CoreEvent::SessionChanged(info) if info.process.is_some() => {
                assert_eq!(info.name, "Marker");
                assert_eq!(info.status, SessionStatus::Stopped);
                assert!(info.pid.is_none());
                stopped = true;
            }
            CoreEvent::ProcessAdded(project) => {
                assert_eq!(project, "test");
                assert!(stopped);
                break;
            }
            CoreEvent::SessionStarted { .. } => panic!("adding a definition must not run it"),
            CoreEvent::ProcessAddFailed { error, .. } | CoreEvent::ConfigError(error) => {
                panic!("{error}")
            }
            _ => {}
        }
    }
    assert!(!directory.join("executed").exists());
    assert_eq!(
        config::read(&path).unwrap().projects[0].processes[0].name,
        "Marker"
    );
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } = next() {
            assert!(clean);
            break;
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
