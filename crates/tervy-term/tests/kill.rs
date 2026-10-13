use std::collections::HashMap;
use std::time::{Duration, Instant};
use tervy_term::kill::{OwnedSession, ProcessIdentity};
use tervy_term::{Dimensions, SpawnSpec, TerminalSession};

fn spawn(program: &str, args: Vec<String>) -> TerminalSession {
    TerminalSession::spawn(
        SpawnSpec {
            program: program.into(),
            args,
            cwd: std::env::temp_dir(),
            env: HashMap::new(),
            scrollback_lines: 100,
        },
        Dimensions::default(),
    )
    .unwrap()
    .0
}
fn wait_empty(owned: OwnedSession) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !owned.members().unwrap().is_empty() {
        assert!(
            Instant::now() < deadline,
            "session members survived SIGKILL"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn sweep_finds_background_groups_and_ignores_reused_identity() {
    let session = spawn(
        "/bin/zsh",
        vec![
            "-f".into(),
            "-i".into(),
            "-c".into(),
            "trap '' HUP TERM; (trap '' HUP TERM; printf READY_ONE; exec sleep 1000) & (trap '' HUP TERM; printf READY_TWO; exec sleep 1000) & wait".into(),
        ],
    );
    let owned = session.owned;
    let deadline = Instant::now() + Duration::from_secs(3);
    while owned.members().unwrap().len() < 3 || {
        let text: String = session
            .handle
            .snapshot(&tervy_term::snapshot::Palette::default())
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        !text.contains("READY_ONE") || !text.contains("READY_TWO")
    } {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let wrong = OwnedSession {
        leader: ProcessIdentity {
            started_us: owned.leader.started_us + 1,
            ..owned.leader
        },
    };
    assert!(wrong.members().unwrap().is_empty());
    session.signal(libc::SIGHUP).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !owned.members().unwrap().is_empty(),
        "fixture should ignore SIGHUP"
    );
    session.signal(libc::SIGKILL).unwrap();
    wait_empty(owned);
}
#[test]
fn sweep_finds_survivor_after_its_leader_has_exited() {
    let script = "import os,signal,time\nsignal.signal(signal.SIGHUP,signal.SIG_IGN)\npid=os.fork()\nif pid==0:\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n time.sleep(1000)\nelse:\n time.sleep(.15)\n os._exit(0)";
    let session = spawn(
        "/usr/bin/python3",
        vec!["-u".into(), "-c".into(), script.into()],
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while ProcessIdentity::read(session.pid as i32).is_ok() {
        assert!(Instant::now() < deadline, "leader not reaped");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!session.owned.members().unwrap().is_empty());
    session.signal(libc::SIGKILL).unwrap();
    wait_empty(session.owned);
}

#[test]
fn explicitly_tracked_descendant_sessions_survive_reparenting_and_exclude_siblings() {
    let mut session = spawn("/usr/bin/python3", vec!["-u".into(), "-c".into(), "import os,signal,time\npid=os.fork()\nif pid==0:\n os.setsid()\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n print('DETACHED_READY',flush=True)\n time.sleep(1000)\nelse: time.sleep(1000)".into()]);
    let unrelated = spawn("/bin/sleep", vec!["1000".into()]);
    let deadline = Instant::now() + Duration::from_secs(3);
    let detached = loop {
        let sessions = session.owned.descendant_sessions().unwrap();
        let ready = session
            .handle
            .snapshot(&tervy_term::snapshot::Palette::default())
            .runs
            .iter()
            .any(|run| run.text.contains("DETACHED_READY"));
        if ready && sessions.len() == 1 {
            break sessions;
        }
        assert!(
            Instant::now() < deadline,
            "detached descendant did not start"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_ne!(detached[0].leader, unrelated.owned.leader);
    let wrong = OwnedSession {
        leader: ProcessIdentity {
            started_us: session.owned.leader.started_us + 1,
            ..session.owned.leader
        },
    };
    assert!(wrong.descendant_sessions().is_err());
    session.track_sessions(detached.clone());
    assert_eq!(session.owned_sessions().len(), 2);
    session.signal(libc::SIGTERM).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !session.sessions_empty().unwrap(),
        "child should ignore TERM after reparenting"
    );
    session.signal(libc::SIGKILL).unwrap();
    wait_empty(session.owned);
    wait_empty(detached[0]);
    assert!(session.sessions_empty().unwrap());
    assert!(!unrelated.owned.members().unwrap().is_empty());
    unrelated.signal(libc::SIGKILL).unwrap();
    wait_empty(unrelated.owned);
}
