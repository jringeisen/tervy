//! Removal uses real supervised sessions, but never launches an agent CLI.
use devdesk_core::{
    agent::{Record, Role},
    config,
    store::Store,
    supervisor::{Command, CoreEvent, Supervisor},
};
use devdesk_term::{Dimensions, kill::OwnedSession};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn event(runtime: &Supervisor, deadline: Instant) -> CoreEvent {
    loop {
        if let Ok(event) = runtime.events.try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "removal timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn fixture() -> (PathBuf, PathBuf) {
    let folder = std::env::temp_dir().join(format!("devdesk-remove-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("keep.txt"), "project files stay").unwrap();
    let path = folder.join("config.toml");
    std::fs::write(&path, format!("# keep this comment\n[[projects]]\nid='p'\nname='P'\npath={0:?}\n[[projects.processes]]\nid='worker'\nname='Worker'\ncommand='sleep 120'\nstop_timeout_secs=1\n[[projects]]\nid='q'\nname='Q'\npath={0:?}\n", folder.to_string_lossy())).unwrap();
    (folder, path)
}
fn record(project: &str, folder: &std::path::Path) -> Record {
    Record {
        id: uuid::Uuid::new_v4().to_string(),
        project: project.into(),
        name: "Saved Claude".into(),
        cwd: folder.into(),
        role: Role::Primary,
        session_id: Some(uuid::Uuid::new_v4().to_string()),
    }
}
fn shutdown(runtime: &Supervisor, deadline: Instant) {
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } = event(runtime, deadline) {
            assert!(clean);
            break;
        }
    }
}
#[test]
fn deleting_a_running_process_stops_it_and_preserves_other_config_and_files() {
    let (folder, path) = fixture();
    let runtime = Supervisor::start(path.clone(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let id = loop {
        if let CoreEvent::SessionChanged(info) = event(&runtime, deadline)
            && info.process.is_some()
        {
            break info.id;
        }
    };
    runtime.handle.send(Command::Start(id));
    let owned = loop {
        if let CoreEvent::SessionStarted { info, .. } = event(&runtime, deadline)
            && info.id == id
        {
            break OwnedSession::capture(info.pid.unwrap()).unwrap();
        }
    };
    runtime.handle.send(Command::RemoveSession(id));
    loop {
        if let CoreEvent::SessionClosed(closed) = event(&runtime, deadline)
            && closed == id
        {
            break;
        }
    }
    assert!(owned.members().unwrap().is_empty());
    let saved = config::read(&path).unwrap();
    assert!(saved.projects[0].processes.is_empty());
    assert_eq!(saved.projects[1].id, "q");
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("# keep this comment")
    );
    assert!(folder.join("keep.txt").exists());
    shutdown(&runtime, deadline);
    drop(runtime);
    let store = Store::open(&folder.join("state.db")).unwrap();
    assert!(store.survivors().unwrap().is_empty());
    drop(store);
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn deleting_an_agent_then_its_project_does_not_restore_removed_records() {
    let (folder, path) = fixture();
    let store = Store::open(&folder.join("state.db")).unwrap();
    let agent = record("p", &folder);
    let other = record("q", &folder);
    let remaining = record("p", &folder);
    for record in [&agent, &other, &remaining] {
        store.save_agent(record).unwrap();
    }
    drop(store);
    let runtime = Supervisor::start(path.clone(), None).unwrap();
    runtime.handle.send(Command::Restore);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut agent_id = None;
    let mut project_ids = Vec::new();
    loop {
        match event(&runtime, deadline) {
            CoreEvent::SessionChanged(info) => {
                if info.project == "p" && !project_ids.contains(&info.id) {
                    project_ids.push(info.id);
                }
                if info.agent.is_some_and(|a| a.record.id == agent.id) {
                    agent_id = Some(info.id);
                }
            }
            CoreEvent::Restored { .. } => break,
            _ => {}
        }
    }
    let id = agent_id.unwrap();
    runtime.handle.send(Command::RemoveSession(id));
    loop {
        if let CoreEvent::SessionClosed(closed) = event(&runtime, deadline)
            && closed == id
        {
            break;
        }
    }
    project_ids.retain(|candidate| *candidate != id);
    runtime.handle.send(Command::NewShell {
        project: "p".into(),
        size: Dimensions::default(),
    });
    let (shell, owned) = loop {
        if let CoreEvent::SessionStarted { info, .. } = event(&runtime, deadline) {
            break (info.id, OwnedSession::capture(info.pid.unwrap()).unwrap());
        }
    };
    project_ids.push(shell);
    runtime.handle.send(Command::RemoveProject("p".into()));
    let mut removed = false;
    while !removed || !project_ids.is_empty() {
        match event(&runtime, deadline) {
            CoreEvent::ProjectRemoved(project) => {
                assert_eq!(project, "p");
                removed = true;
            }
            CoreEvent::SessionClosed(id) => project_ids.retain(|candidate| *candidate != id),
            _ => {}
        }
    }
    assert!(owned.members().unwrap().is_empty());
    assert_eq!(
        config::read(&path)
            .unwrap()
            .projects
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["q"]
    );
    assert!(folder.join("keep.txt").exists());
    shutdown(&runtime, deadline);
    drop(runtime);
    let store = Store::open(&folder.join("state.db")).unwrap();
    let records = store.agents().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, other.id);
    drop(store);
    // Relaunch must neither restore removed records nor recreate process rows.
    let runtime = Supervisor::start(path, None).unwrap();
    runtime.handle.send(Command::Restore);
    loop {
        match event(&runtime, deadline) {
            CoreEvent::SessionChanged(info) => {
                assert_eq!(info.project, "q");
                assert_eq!(info.agent.unwrap().record.id, other.id);
            }
            CoreEvent::Restored { .. } => break,
            _ => {}
        }
    }
    shutdown(&runtime, deadline);
    drop(runtime);
    std::fs::remove_dir_all(folder).unwrap();
}
