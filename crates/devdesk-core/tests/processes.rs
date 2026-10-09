use devdesk_core::{
    store::Store,
    supervisor::{Command, CoreEvent, SessionId, SessionStatus, Supervisor},
};
use devdesk_term::{TermMode, kill::OwnedSession};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
fn next(runtime: &Supervisor, deadline: Instant) -> CoreEvent {
    loop {
        if let Ok(event) = runtime.events.try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "process supervisor timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn config(name: &str, program: &str, restart: bool) -> (PathBuf, PathBuf) {
    let folder = std::env::temp_dir().join(format!("devdesk-m3-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let script = folder.join("worker.py");
    std::fs::write(&script, program).unwrap();
    let path = folder.join("config.toml");
    std::fs::write(&path,format!("[[projects]]\nid='p'\nname='P'\npath={:?}\n[[projects.processes]]\nid='worker'\nname='Worker'\ncommand=\"/usr/bin/python3 -u worker.py\"\nstop_timeout_secs=1\nrestart={{policy='{}',max_retries=5}}\n",folder.to_string_lossy(),if restart {"on-failure"}else{"never"})).unwrap();
    (folder, path)
}
fn configured(runtime: &Supervisor, deadline: Instant) -> SessionId {
    loop {
        if let CoreEvent::SessionChanged(info) = next(runtime, deadline)
            && info.process.as_deref() == Some("worker")
        {
            assert_eq!(info.status, SessionStatus::Stopped);
            return info.id;
        }
    }
}
fn shutdown(runtime: &Supervisor, deadline: Instant) {
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } = next(runtime, deadline) {
            assert!(clean);
            break;
        }
    }
}
#[test]
fn restart_reuses_grid_preserves_alt_crash_and_rebinds_terminal_replies() {
    let (folder, path) = config(
        "history",
        r#"import os,pathlib,tty,time
marker=pathlib.Path('ran')
if not marker.exists():
 marker.touch()
 os.write(1,b'BEFORE_CRASH\r\n\x1b[?1049h\x1b[?1003h\x1b[?1006h\x1b[?1004h\x1b[?2004h\x1b[>31uALT_CRASH\r\n')
 raise SystemExit(7)
tty.setraw(0)
os.write(1,b'\x1b[6n')
reply=b''
while not reply.endswith(b'R'): reply+=os.read(0,1)
assert reply.startswith(b'\x1b['),reply
os.write(1,b'AFTER_RESTART\r\n')
"#,
        false,
    );
    let runtime = Supervisor::start(path, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let id = configured(&runtime, deadline);
    runtime.handle.send(Command::Start(id));
    let first = loop {
        if let CoreEvent::SessionStarted { handle, .. } = next(&runtime, deadline) {
            break handle;
        }
    };
    loop {
        if let CoreEvent::SessionChanged(info) = next(&runtime, deadline)
            && info.id == id
            && matches!(info.status, SessionStatus::Failed(_))
        {
            break;
        }
    }
    assert!(first.mode().contains(
        TermMode::ALT_SCREEN
            | TermMode::MOUSE_MOTION
            | TermMode::BRACKETED_PASTE
            | TermMode::KITTY_KEYBOARD_PROTOCOL
    ));
    runtime.handle.send(Command::Restart(id));
    let second = loop {
        if let CoreEvent::SessionStarted { handle, .. } = next(&runtime, deadline) {
            break handle;
        }
    };
    assert!(Arc::ptr_eq(&first.term, &second.term));
    loop {
        if let CoreEvent::SessionChanged(info) = next(&runtime, deadline)
            && info.id == id
            && info.status == SessionStatus::Stopped
        {
            break;
        }
    }
    assert_eq!(second.mode(), TermMode::default());
    let snapshot = second.snapshot(&devdesk_term::snapshot::Palette::default());
    let text = snapshot
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    for marker in [
        "BEFORE_CRASH",
        "ALT_CRASH",
        "restarted after exit 7",
        "AFTER_RESTART",
    ] {
        assert!(text.contains(marker), "missing {marker}: {text}");
    }
    shutdown(&runtime, deadline);
    drop(runtime);
    std::fs::remove_dir_all(folder).unwrap();
}
#[test]
fn restart_waits_for_leaderless_port_holder_and_config_applies_only_on_restart() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let program = format!(
        r#"import os,pathlib,socket,signal,time
s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',{port}));s.listen()
marker=pathlib.Path('port-ready')
if not marker.exists():
 if os.fork()==0:
  for sig in (signal.SIGHUP,signal.SIGINT,signal.SIGTERM):signal.signal(sig,signal.SIG_IGN)
  marker.write_text(str(os.getpid()))
  time.sleep(1000)
 else:
  while not marker.exists():time.sleep(.01)
  raise SystemExit(4)
else:
 pathlib.Path('rebound').touch()
 time.sleep(1000)
"#
    );
    let (folder, path) = config("port", &program, false);
    let runtime = Supervisor::start(path.clone(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let id = configured(&runtime, deadline);
    runtime.handle.send(Command::Start(id));
    let owned = loop {
        if let CoreEvent::SessionStarted { info, .. } = next(&runtime, deadline) {
            break OwnedSession::capture(info.pid.unwrap()).unwrap();
        }
    };
    while !folder.join("port-ready").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        old.replace("stop_timeout_secs=1", "stop_timeout_secs=2"),
    )
    .unwrap();
    loop {
        if let CoreEvent::SessionChanged(info) = next(&runtime, deadline)
            && info.id == id
            && info.config_changed
        {
            break;
        }
    }
    runtime.handle.send(Command::Restart(id));
    loop {
        if let CoreEvent::SessionStarted { info, .. } = next(&runtime, deadline) {
            assert_ne!(info.pid, Some(owned.leader.pid as u32));
            assert!(!info.config_changed);
            break;
        }
    }
    assert!(
        owned.members().unwrap().is_empty(),
        "old port holder survived restart"
    );
    while !folder.join("rebound").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    shutdown(&runtime, deadline);
    drop(runtime);
    std::fs::remove_dir_all(folder).unwrap();
}

// Runs only as a dedicated subprocess of the crash-recovery test. Its parent
// SIGKILLs this process after Running; no destructors can repair the ledger.
#[test]
#[ignore]
fn ledger_crash_child() {
    let Some(path) = std::env::var_os("DEVDESK_TEST_CRASH_CONFIG") else {
        return;
    };
    let runtime = Supervisor::start(path.into(), None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let id = configured(&runtime, deadline);
    runtime.handle.send(Command::Start(id));
    loop {
        let _ = next(&runtime, Instant::now() + Duration::from_secs(3600));
    }
}
struct SurvivorGuard(OwnedSession);
impl Drop for SurvivorGuard {
    fn drop(&mut self) {
        if let Ok(members) = self.0.members() {
            for member in members {
                let _ = member.signal(libc::SIGKILL, self.0.leader.pid);
            }
        }
    }
}
#[test]
fn force_killed_owner_leaves_recoverable_ledger_and_relaunch_stops_survivors() {
    let (folder, path) = config(
        "recovery",
        r#"import os,signal,pathlib,time
for sig in (signal.SIGHUP,signal.SIGINT,signal.SIGTERM):signal.signal(sig,signal.SIG_IGN)
if os.fork()==0:
 pathlib.Path('crash-ready').touch()
 time.sleep(1000)
else:
 while not pathlib.Path('crash-ready').exists():time.sleep(.01)
 raise SystemExit(4)
"#,
        false,
    );
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "ledger_crash_child", "--nocapture"])
        .env("DEVDESK_TEST_CRASH_CONFIG", &path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !folder.join("crash-ready").exists() {
        assert!(Instant::now() < deadline);
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    let store = Store::open(&folder.join("state.db")).unwrap();
    let before = store.survivors().unwrap();
    assert_eq!(before.len(), 1);
    let guard = SurvivorGuard(before[0].session);
    while devdesk_term::kill::ProcessIdentity::read(guard.0.leader.pid).is_ok() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!guard.0.members().unwrap().is_empty());
    let runtime = Supervisor::start(path, None).unwrap();
    loop {
        if let CoreEvent::Orphans(orphans) = next(&runtime, deadline) {
            assert_eq!(orphans.len(), 1);
            assert_eq!(orphans[0].owner, "process:p/worker");
            break;
        }
    }
    runtime.handle.send(Command::ResolveOrphans { kill: true });
    while !guard.0.members().unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    shutdown(&runtime, deadline);
    drop(runtime);
    assert!(store.survivors().unwrap().is_empty());
    drop(store);
    drop(guard);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn automatic_retry_keeps_history_and_stop_cancels_further_retries() {
    let (folder, path) = config(
        "automatic",
        r#"import pathlib,time
p=pathlib.Path('attempts'); n=int(p.read_text())+1 if p.exists() else 1;p.write_text(str(n))
print('ATTEMPT_'+str(n),flush=True)
if n<3:raise SystemExit(7)
time.sleep(1000)
"#,
        true,
    );
    let runtime = Supervisor::start(path, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    let id = configured(&runtime, deadline);
    runtime.handle.send(Command::Start(id));
    let mut grids = vec![];
    let mut last_owned = None;
    while grids.len() < 3 {
        if let CoreEvent::SessionStarted { info, handle, .. } = next(&runtime, deadline) {
            last_owned = Some(OwnedSession::capture(info.pid.unwrap()).unwrap());
            grids.push(handle);
        }
    }
    assert!(
        grids
            .iter()
            .all(|handle| Arc::ptr_eq(&handle.term, &grids[0].term))
    );
    let ready = folder.join("attempts");
    while std::fs::read_to_string(&ready).unwrap_or_default() != "3" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    runtime.handle.send(Command::Stop(id));
    loop {
        if let CoreEvent::SessionChanged(info) = next(&runtime, deadline)
            && info.id == id
            && info.status == SessionStatus::Stopped
        {
            break;
        }
    }
    assert!(last_owned.unwrap().members().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(std::fs::read_to_string(ready).unwrap(), "3");
    shutdown(&runtime, deadline);
    drop(runtime);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn project_autostart_occurs_once_and_start_all_stop_all_are_idempotent() {
    let (folder, path) = config("autostart", "import time;time.sleep(1000)", false);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("stop_timeout_secs=1", "autostart=true\nstop_timeout_secs=1");
    std::fs::write(&path, text).unwrap();
    let runtime = Supervisor::start(path, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let id = configured(&runtime, deadline);
    runtime.handle.send(Command::ProjectOpened("p".into()));
    let first = loop {
        if let CoreEvent::SessionStarted { info, .. } = next(&runtime, deadline) {
            break info.pid;
        }
    };
    runtime.handle.send(Command::StartAll("p".into()));
    runtime.handle.send(Command::ProjectOpened("p".into()));
    std::thread::sleep(Duration::from_millis(150));
    while let Ok(event) = runtime.events.try_recv() {
        assert!(
            !matches!(event, CoreEvent::SessionStarted { .. }),
            "already-running process was duplicated"
        );
    }
    runtime.handle.send(Command::StopAll("p".into()));
    loop {
        if let CoreEvent::SessionChanged(info) = next(&runtime, deadline)
            && info.id == id
            && info.status == SessionStatus::Stopped
        {
            break;
        }
    }
    runtime.handle.send(Command::ProjectOpened("p".into()));
    std::thread::sleep(Duration::from_millis(150));
    while let Ok(event) = runtime.events.try_recv() {
        assert!(
            !matches!(event, CoreEvent::SessionStarted { .. }),
            "second project open autostarted again"
        );
    }
    runtime.handle.send(Command::StartAll("p".into()));
    loop {
        if let CoreEvent::SessionStarted { info, .. } = next(&runtime, deadline) {
            assert_ne!(info.pid, first);
            break;
        }
    }
    shutdown(&runtime, deadline);
    drop(runtime);
    std::fs::remove_dir_all(folder).unwrap();
}
