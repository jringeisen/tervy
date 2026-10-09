//! Real PTYs and the supervisor/provider path, using a synthetic CLI that only
//! reports documented hook fields. The subprocess isolates its shell environment
//! from Rust's parallel test runner; this test never contacts a model service.
use devdesk_core::{
    agent::Role,
    store::Store,
    supervisor::{Command, CoreEvent, SessionId, SessionStatus, Supervisor},
};
use devdesk_term::kill::OwnedSession;
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn event(runtime: &Supervisor, deadline: Instant) -> CoreEvent {
    loop {
        if let Ok(event) = runtime.events.try_recv() {
            return event;
        }
        assert!(Instant::now() < deadline, "fork fixture timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn running(
    runtime: &Supervisor,
    deadline: Instant,
    side: bool,
) -> (SessionId, OwnedSession, String) {
    let mut owned = None;
    loop {
        match event(runtime, deadline) {
            CoreEvent::SessionStarted { info, .. } => {
                let session_owned = OwnedSession::capture(info.pid.unwrap()).unwrap();
                owned = Some(session_owned);
                if let Some(agent) = info.agent
                    && (agent.record.role == Role::Side) == side
                    && agent.status != devdesk_core::agent::Status::Launching
                    && let Some(session) = agent.record.session_id
                {
                    return (info.id, session_owned, session);
                }
            }
            CoreEvent::SessionChanged(info) if info.status == SessionStatus::Running => {
                if let Some(agent) = info.agent
                    && (agent.record.role == Role::Side) == side
                    && agent.status != devdesk_core::agent::Status::Launching
                    && let Some(session) = agent.record.session_id
                    && let Some(owned) = owned
                {
                    return (info.id, owned, session);
                }
            }
            CoreEvent::ConfigError(error) => panic!("{error}"),
            _ => {}
        }
    }
}
fn fixture(folder: &Path) {
    let path = folder.join("config.toml");
    std::fs::write(
        &path,
        format!(
            "[[projects]]\nid='p'\nname='P'\npath={:?}\n",
            folder.to_string_lossy()
        ),
    )
    .unwrap();
    let runtime = Supervisor::start(path, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    runtime.handle.send(Command::NewAgent {
        project: "p".into(),
        picker: false,
    });
    let (parent, parent_owned, parent_session) = running(&runtime, deadline, false);
    runtime.handle.send(Command::ForkAgent {
        parent,
        question: "Why this choice?".into(),
    });
    let (side, side_owned, side_session) = running(&runtime, deadline, true);
    assert_ne!(side_session, parent_session);
    let store = Store::open(&folder.join("state.db")).unwrap();
    let records = store.agents().unwrap();
    let side_record = records
        .iter()
        .find(|record| record.role == Role::Side)
        .unwrap();
    let parent_record = records
        .iter()
        .find(|record| record.role == Role::Primary)
        .unwrap();
    assert_eq!(side_record.parent.as_ref(), Some(&parent_record.id));
    assert_eq!(side_record.cwd, parent_record.cwd);
    assert!(side_record.snapshot_at.is_some());
    let argv_path = folder.join(format!("{}.argv", side_record.id));
    let argv: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(&argv_path).unwrap()).unwrap();
    assert_eq!(&argv[..3], ["--resume", &parent_session, "--fork-session"]);
    assert!(argv.iter().any(|arg| arg == "--restricted"));
    assert!(argv.last().unwrap().contains("Question: Why this choice?"));
    runtime.handle.send(Command::Close {
        id: side,
        confirmed: None,
    });
    loop {
        if let CoreEvent::SessionChanged(info) = event(&runtime, deadline)
            && info.id == side
            && matches!(
                info.status,
                SessionStatus::Stopped | SessionStatus::Exited { .. }
            )
        {
            break;
        }
    }
    assert!(side_owned.members().unwrap().is_empty());
    assert!(!parent_owned.members().unwrap().is_empty());
    runtime.handle.send(Command::ResumeAgent(side));
    let (_, resumed_owned, resumed_session) = running(&runtime, deadline, true);
    assert_eq!(resumed_session, side_session);
    let argv: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(argv_path).unwrap()).unwrap();
    assert_eq!(&argv[..2], ["--resume", &side_session]);
    assert!(!argv.iter().any(|arg| arg == "--fork-session"));
    for flag in [
        "--restricted",
        "--strict-mcp-config",
        "--tools=Read,Grep,Glob",
        "--permission-mode=dontAsk",
    ] {
        assert!(argv.iter().any(|arg| arg == flag));
    }
    runtime.handle.send(Command::RemoveSession(parent));
    let mut remaining = vec![parent, side];
    while !remaining.is_empty() {
        if let CoreEvent::SessionClosed(id) = event(&runtime, deadline) {
            remaining.retain(|candidate| *candidate != id);
        }
    }
    assert!(parent_owned.members().unwrap().is_empty());
    assert!(resumed_owned.members().unwrap().is_empty());
    assert!(store.agents().unwrap().is_empty());
    runtime.handle.send(Command::Shutdown);
    loop {
        if let CoreEvent::ShutdownComplete { clean } = event(&runtime, deadline) {
            assert!(clean);
            break;
        }
    }
}

#[test]
fn forks_close_independently_resume_read_only_and_remove_with_parent() {
    if let Some(folder) = std::env::var_os("DEVDESK_FORK_FIXTURE") {
        fixture(Path::new(&folder));
        return;
    }
    use std::{
        os::unix::fs::PermissionsExt,
        process::{Command as ProcessCommand, Stdio},
    };
    let folder = std::env::temp_dir().join(format!("devdesk-fork-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&folder).unwrap();
    let cli = folder.join("claude");
    std::fs::write(&cli, r#"#!/usr/bin/python3
import json,os,socket,sys,time,uuid
if '--version' in sys.argv: print('2.1.295'); sys.exit(0)
a=sys.argv[1:]
with open(os.environ['DEVDESK_AGENT_ID']+'.argv','w') as f: json.dump(a,f)
fork='--fork-session' in a
sid=str(uuid.uuid4()) if fork else a[a.index('--session-id')+1] if '--session-id' in a else a[a.index('--resume')+1]
p={'agent_id':os.environ['DEVDESK_AGENT_ID'],'launch':int(os.environ['DEVDESK_LAUNCH_ID']),'token':os.environ['DEVDESK_HOOK_TOKEN'],'event':'SessionStart','session_id':sid,'source':'fork' if fork else 'startup','notification_type':None,'tool_use_id':None,'tool_name':None,'stop_hook_active':False}
s=socket.socket(socket.AF_UNIX); s.connect(os.environ['DEVDESK_HOOK_SOCKET']); s.sendall((json.dumps(p)+'\n').encode()); s.close()
while True: print('synthetic CLI alive',flush=True); time.sleep(0.2)
"#).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let shell = folder.join("fixture-shell");
    std::fs::write(
        &shell,
        "#!/bin/sh\nprintf '\\0DEVDESK_ENV\\0'\n/usr/bin/env -0\n",
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
    let log = folder.join("test.log");
    let stdout = std::fs::File::create(&log).unwrap();
    let mut child = ProcessCommand::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "forks_close_independently_resume_read_only_and_remove_with_parent",
            "--nocapture",
        ])
        .env("DEVDESK_FORK_FIXTURE", &folder)
        .env("SHELL", shell)
        .env("PATH", format!("{}:/usr/bin:/bin", folder.display()))
        .stdout(stdout.try_clone().unwrap())
        .stderr(stdout)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("fork subprocess timed out; {}", log.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let output = std::fs::read_to_string(&log).unwrap();
    assert!(status.success(), "{output}");
    std::fs::remove_dir_all(folder).unwrap();
}
