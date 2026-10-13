use std::time::{Duration, Instant};
use tervy_core::{
    agent::{Record, Role, Status},
    layout::{Kind, Layout, Tab},
    store::Store,
    supervisor::{Command, CoreEvent, SessionStatus, Supervisor},
};
fn event(runtime: &Supervisor, deadline: Instant) -> CoreEvent {
    loop {
        if let Ok(event) = runtime.events.try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "restore timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn restores_shell_cwd_and_agent_stub_without_launching_primary_or_side_records() {
    let folder = std::env::temp_dir().join(format!("tervy-restore-{}", std::process::id()));
    let nested = folder.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let path = folder.join("config.toml");
    std::fs::write(
        &path,
        format!(
            "[[projects]]\nid='p'\nname='P'\npath={:?}\n",
            folder.to_string_lossy()
        ),
    )
    .unwrap();
    let store = Store::open(&folder.join("state.db")).unwrap();
    let record = Record {
        id: uuid::Uuid::new_v4().to_string(),
        project: "p".into(),
        name: "Saved Claude".into(),
        cwd: folder.clone(),
        role: Role::Primary,
        launch_args: vec![],
        provider: tervy_core::agent::Provider::Claude,
        parent: None,
        snapshot_at: None,
        session_id: Some(uuid::Uuid::new_v4().to_string()),
    };
    store.save_agent(&record).unwrap();
    store
        .save_agent(&Record {
            id: uuid::Uuid::new_v4().to_string(),
            role: Role::Side,
            launch_args: vec![],
            provider: tervy_core::agent::Provider::Claude,
            parent: Some(record.id.clone()),
            snapshot_at: Some(1_791_518_400),
            ..record.clone()
        })
        .unwrap();
    let key = format!("agent:{}", record.id);
    let mut layout = Layout {
        sidebar_width: Some(320.),
        sidebar_collapsed: false,
        active_project: Some("p".into()),
        selected: vec![key.clone()],
        expanded: vec!["p".into()],
        tabs: vec![
            Tab {
                key: "shell:old".into(),
                project: "p".into(),
                name: "Saved shell".into(),
                kind: Kind::Shell,
                cwd: Some(nested.clone()),
                open: true,
            },
            Tab {
                key: key.clone(),
                project: "p".into(),
                name: record.name.clone(),
                kind: Kind::Agent(record.id.clone()),
                cwd: None,
                open: true,
            },
        ],
        ..Default::default()
    };
    store.save_layout(&layout).unwrap();
    drop(store);
    let runtime = Supervisor::start(path, None).unwrap();
    runtime.handle.send(Command::Restore);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut restored = None;
    let mut shell = None;
    let mut agents = 0;
    while restored.is_none() || shell.is_none() {
        match event(&runtime, deadline) {
            CoreEvent::SessionChanged(info) if info.agent.is_some() => {
                let agent = info.agent.unwrap();
                if agent.record.role == Role::Primary {
                    assert_eq!(agent.record.id, record.id);
                    assert_eq!(agent.record.session_id, record.session_id);
                }
                assert_eq!(agent.status, Status::Exited);
                assert!(matches!(info.status, SessionStatus::Exited { .. }));
                agents += 1;
            }
            CoreEvent::SessionStarted { info, .. } => {
                assert!(
                    info.agent.is_none(),
                    "Restore must not resume Claude automatically"
                );
                let owned = tervy_term::kill::OwnedSession::capture(info.pid.unwrap()).unwrap();
                assert_eq!(
                    tervy_term::kill::cwd(owned.leader)
                        .unwrap()
                        .canonicalize()
                        .unwrap(),
                    nested.canonicalize().unwrap()
                );
                shell = Some(info.id);
            }
            CoreEvent::Restored {
                saved,
                layout,
                keys,
            } => {
                assert!(saved);
                assert_eq!(layout.selected, vec![key.clone()]);
                assert!(keys.contains_key(&key));
                restored = Some(keys);
            }
            _ => {}
        }
    }
    assert_eq!(agents, 2);
    layout.tabs[0].key = format!("shell:{}", shell.unwrap().0);
    layout.tabs[0].cwd = None;
    runtime.handle.send(Command::SaveLayout(layout));
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } = event(&runtime, deadline) {
            assert!(clean);
            break;
        }
    }
    drop(runtime);
    let store = Store::open(&folder.join("state.db")).unwrap();
    let saved = store.layout().unwrap().unwrap();
    assert_eq!(
        saved.tabs[0].cwd.as_ref().unwrap().canonicalize().unwrap(),
        nested.canonicalize().unwrap()
    );
    assert_eq!(store.agents().unwrap().len(), 2);
    drop(store);
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn cwd_query_rejects_changed_birth_time() {
    let mut identity = tervy_term::kill::ProcessIdentity::read(std::process::id() as i32).unwrap();
    assert_eq!(
        tervy_term::kill::cwd(identity)
            .unwrap()
            .canonicalize()
            .unwrap(),
        std::env::current_dir().unwrap().canonicalize().unwrap()
    );
    identity.started_us += 1;
    assert!(tervy_term::kill::cwd(identity).is_err());
}
