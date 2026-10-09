//! No-model Codex lifecycle fixture: native TUI/job session separation only.
use devdesk_core::{
    agent::Provider,
    store::Store,
    supervisor::{Command, CoreEvent, Supervisor},
};
use devdesk_term::kill::OwnedSession;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
#[test]
#[ignore]
fn journal_child() {
    let Some(folder) = std::env::var_os("DEVDESK_JOB_FIXTURE") else {
        return;
    };
    let runtime = Supervisor::start(PathBuf::from(folder).join("config.toml"), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut launched = false;
    loop {
        if let Ok(event) = runtime.events.try_recv() {
            match event {
                CoreEvent::Config(_) if !launched => {
                    launched = true;
                    runtime.handle.send(Command::NewAgent {
                        project: "p".into(),
                        picker: false,
                        provider: Provider::Codex,
                    });
                }
                CoreEvent::SessionStarted { info, .. } => {
                    runtime.handle.send(Command::AgentInput {
                        id: info.id,
                        launch: info.launch,
                        enter: true,
                    })
                }
                CoreEvent::ConfigError(error) => panic!("{error}"),
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "fixture timeout");
        std::thread::sleep(Duration::from_millis(5));
    }
}
struct Cleanup(Vec<OwnedSession>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        for owned in &self.0 {
            if let Ok(members) = owned.members() {
                for member in members {
                    let _ = member.signal(libc::SIGKILL, owned.leader.pid);
                }
            }
        }
    }
}
#[test]
fn crash_journal_recovers_codex_job_after_tui_dies() {
    use std::os::unix::fs::PermissionsExt;
    let folder = std::env::temp_dir().join(format!("devdesk-job-{}", std::process::id()));
    std::fs::create_dir_all(folder.join("bin")).unwrap();
    std::fs::create_dir_all(folder.join("codex-home")).unwrap();
    std::fs::write(folder.join("codex-home/config.toml"), "").unwrap();
    std::fs::write(
        folder.join("config.toml"),
        format!(
            "[[projects]]\nid='p'\nname='P'\npath={:?}\n",
            folder.to_string_lossy()
        ),
    )
    .unwrap();
    let cli = folder.join("bin/codex");
    std::fs::write(
        &cli,
        r#"#!/usr/bin/python3
import os, sys, time, signal, pathlib
if '--version' in sys.argv:
 print('codex-cli 0.162.0')
 raise SystemExit
if os.fork()==0:
 os.setsid()
 signal.signal(signal.SIGTERM, signal.SIG_IGN)
 signal.signal(signal.SIGHUP, signal.SIG_IGN)
 pathlib.Path('job-ready').touch()
 time.sleep(60)
else:
 while True: time.sleep(1)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let shell = folder.join("shell");
    std::fs::write(
        &shell,
        "#!/bin/sh\nprintf '\\0DEVDESK_ENV\\0'\n/usr/bin/env -0\n",
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "journal_child", "--ignored", "--nocapture"])
        .env("DEVDESK_JOB_FIXTURE", &folder)
        .env("CODEX_HOME", folder.join("codex-home"))
        .env("SHELL", shell)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", folder.join("bin").display()),
        )
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !folder.join("job-ready").exists() {
        assert!(Instant::now() < deadline);
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    let store = Store::open(&folder.join("state.db")).unwrap();
    let sessions = loop {
        let sessions = store.survivors().unwrap();
        if sessions.len() == 2 {
            break sessions;
        }
        assert!(Instant::now() < deadline, "active job was never journaled");
        std::thread::sleep(Duration::from_millis(20));
    };
    let cleanup = Cleanup(sessions.iter().map(|orphan| orphan.session).collect());
    let root = sessions
        .iter()
        .find(|orphan| {
            orphan
                .session
                .descendant_sessions()
                .is_ok_and(|jobs| !jobs.is_empty())
        })
        .unwrap()
        .session;
    child.kill().unwrap();
    child.wait().unwrap();
    // The job's own SID survives even when its TUI is forcibly removed.
    root.leader.signal(libc::SIGKILL, root.leader.pid).unwrap();
    while !root.members().unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(100));
    let remaining = store.survivors().unwrap();
    assert!(!remaining.is_empty());
    let runtime = Supervisor::start(folder.join("config.toml"), None).unwrap();
    loop {
        if let Ok(CoreEvent::Orphans(orphans)) = runtime.events.try_recv() {
            assert!(!orphans.is_empty());
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    runtime.handle.send(Command::ResolveOrphans { kill: true });
    while cleanup
        .0
        .iter()
        .any(|owned| !owned.members().unwrap().is_empty())
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    runtime.handle.send(Command::Shutdown);
    drop(runtime);
    assert!(store.survivors().unwrap().is_empty());
    drop(store);
    drop(cleanup);
    std::fs::remove_dir_all(folder).unwrap();
}
