use devdesk_core::supervisor::{Command, CoreEvent, Supervisor};
use devdesk_term::{Dimensions, kill::OwnedSession};
use std::time::{Duration, Instant};
fn next(runtime: &Supervisor, deadline: Instant) -> CoreEvent {
    loop {
        if let Ok(event) = runtime.events.try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "supervisor timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn live_shell_publishes_memory_with_its_current_launch_identity() {
    let directory = std::env::temp_dir().join(format!("devdesk-memory-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("config.toml");
    std::fs::write(
        &path,
        format!(
            "[[projects]]\nid='test'\nname='Memory'\npath='{}'\n",
            directory.display()
        ),
    )
    .unwrap();
    let runtime = Supervisor::start(path, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !matches!(next(&runtime, deadline), CoreEvent::Config(_)) {}
    runtime.handle.send(Command::NewShell {
        project: "test".into(),
        size: Dimensions::default(),
    });
    let info = loop {
        if let CoreEvent::SessionStarted { info, .. } = next(&runtime, deadline) {
            break info;
        }
    };
    loop {
        if let CoreEvent::MemoryUsage(samples) = next(&runtime, deadline)
            && samples.iter().any(|(id, launch, bytes)| {
                *id == info.id && *launch == info.launch && bytes.is_some_and(|bytes| bytes > 0)
            })
        {
            break;
        }
    }
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } = next(&runtime, deadline) {
            assert!(clean);
            break;
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn ten_busy_shells_shutdown_in_parallel_and_config_reload_keeps_last_good() {
    let directory = std::env::temp_dir().join(format!("devdesk-supervisor-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("config.toml");
    let text = format!(
        "# owner comment\n[[projects]]\nid='test'\nname='Before'\npath='{}'\n",
        directory.display()
    );
    std::fs::write(&path, &text).unwrap();
    let runtime = Supervisor::start(path.clone(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !matches!(next(&runtime, deadline), CoreEvent::Config(_)) {}
    for _ in 0..10 {
        runtime.handle.send(Command::NewShell {
            project: "test".into(),
            size: Dimensions::default(),
        });
    }
    let mut sessions = vec![];
    while sessions.len() < 10 {
        if let CoreEvent::SessionStarted { info, handle, .. } = next(&runtime, deadline) {
            let owned = OwnedSession::capture(info.pid.unwrap()).unwrap();
            let ready = directory.join(format!("ready-{}", info.id.0));
            let program = format!(
                "import signal,os,time\nsignal.signal(signal.SIGHUP,signal.SIG_IGN)\nsignal.signal(signal.SIGTERM,signal.SIG_IGN)\nfor _ in range(2):\n if os.fork()==0: break\nopen({:?},'w').write('ready')\ntime.sleep(1000)",
                ready.to_string_lossy()
            );
            handle.write(
                format!("/usr/bin/python3 -c '{}'\n", program.replace('\'', "'\\''")).into_bytes(),
            );
            sessions.push((owned, ready));
        }
    }
    while !sessions.iter().all(|(_, path)| path.exists()) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::write(&path, text.replace("Before", "After")).unwrap();
    let reload = Instant::now();
    loop {
        if let CoreEvent::Config(config) = next(&runtime, deadline)
            && config.projects[0].name == "After"
        {
            break;
        }
    }
    assert!(
        reload.elapsed() < Duration::from_secs(1),
        "hot reload exceeded one second"
    );
    std::fs::write(&path, "this is not TOML").unwrap();
    while !matches!(next(&runtime, deadline), CoreEvent::ConfigError(_)) {}
    // A new shell still uses the last good project despite the invalid file.
    runtime.handle.send(Command::NewShell {
        project: "test".into(),
        size: Dimensions::default(),
    });
    while !matches!(next(&runtime, deadline), CoreEvent::SessionStarted { .. }) {}
    let start = Instant::now();
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } =
            next(&runtime, start + Duration::from_secs(15))
        {
            assert!(clean);
            break;
        }
    }
    assert!(start.elapsed() < Duration::from_secs(15));
    for (owned, _) in sessions {
        assert!(
            owned.members().unwrap().is_empty(),
            "session survived shutdown"
        );
    }
    drop(runtime);
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn stop_requested_during_start_does_not_leave_a_late_child() {
    let runtime = Supervisor::start(
        std::env::temp_dir().join("unused-devdesk-probe.toml"),
        Some("shell".into()),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !matches!(next(&runtime, deadline), CoreEvent::Config(_)) {}
    runtime.handle.send(Command::NewShell {
        project: "probe".into(),
        size: Dimensions::default(),
    });
    let id = loop {
        if let CoreEvent::SessionChanged(info) = next(&runtime, deadline) {
            break info.id;
        }
    };
    runtime.handle.send(Command::Close {
        id,
        confirmed: None,
    });
    let mut owned = None;
    loop {
        match next(&runtime, deadline) {
            CoreEvent::SessionStarted { info, .. } => {
                owned = OwnedSession::capture(info.pid.unwrap()).ok();
            }
            CoreEvent::SessionClosed(closed) if closed == id => break,
            _ => {}
        }
    }
    if let Some(owned) = owned {
        assert!(owned.members().unwrap().is_empty());
    }
    runtime.handle.send(Command::Shutdown);
}

#[test]
fn foreground_close_requires_confirmation_for_the_current_launch() {
    let runtime = Supervisor::start(
        std::env::temp_dir().join("unused-devdesk-confirm.toml"),
        Some("shell".into()),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !matches!(next(&runtime, deadline), CoreEvent::Config(_)) {}
    runtime.handle.send(Command::NewShell {
        project: "probe".into(),
        size: Dimensions::default(),
    });
    let (info, handle) = loop {
        if let CoreEvent::SessionStarted { info, handle, .. } = next(&runtime, deadline) {
            break (info, handle);
        }
    };
    let owned = OwnedSession::capture(info.pid.unwrap()).unwrap();
    let ready = std::env::temp_dir().join(format!("devdesk-confirm-{}", info.pid.unwrap()));
    let program = format!(
        "import pathlib,time; pathlib.Path({:?}).touch(); time.sleep(1000)",
        ready.to_string_lossy()
    );
    handle
        .write(format!("/usr/bin/python3 -c '{}'\n", program.replace('\'', "'\\''")).into_bytes());
    while !ready.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    // Neither an unconfirmed request nor an approval for an older launch may
    // stop this foreground job. Both must produce a fresh confirmation.
    for confirmed in [None, Some(devdesk_core::supervisor::LaunchId(0))] {
        runtime.handle.send(Command::Close {
            id: info.id,
            confirmed,
        });
        loop {
            match next(&runtime, deadline) {
                CoreEvent::ConfirmClose { id, launch, job } => {
                    assert_eq!(id, info.id);
                    assert_eq!(launch, info.launch);
                    assert!(!job.is_empty());
                    break;
                }
                CoreEvent::SessionChanged(changed) => assert_ne!(
                    changed.status,
                    devdesk_core::supervisor::SessionStatus::Stopping
                ),
                CoreEvent::SessionClosed(_) => panic!("closed without current approval"),
                _ => {}
            }
        }
        assert!(!owned.members().unwrap().is_empty());
    }
    runtime.handle.send(Command::Close {
        id: info.id,
        confirmed: Some(info.launch),
    });
    while !matches!(next(&runtime, deadline), CoreEvent::SessionClosed(id) if id == info.id) {}
    assert!(owned.members().unwrap().is_empty());
    std::fs::remove_file(ready).unwrap();
    runtime.handle.send(Command::Shutdown);
}
