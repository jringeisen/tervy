use std::collections::HashMap;
use std::time::{Duration, Instant};
use tervy_term::snapshot::Palette;
use tervy_term::{Dimensions, SpawnSpec, TerminalSession, TerminalSignal};

fn spawn(command: &str) -> (TerminalSession, tervy_term::TerminalEvents) {
    TerminalSession::spawn(
        SpawnSpec {
            program: "/bin/zsh".into(),
            args: vec!["-c".into(), command.into()],
            cwd: std::env::temp_dir(),
            env: HashMap::new(),
            scrollback_lines: 100,
        },
        Dimensions::default(),
    )
    .unwrap()
}

#[test]
fn memory_samples_include_children_and_reject_reused_birth_identity() {
    let (session, _events) = spawn("sleep 30 & printf 'READY\\n'; wait");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let members = session.owned.members().unwrap();
        let leader = session.owned.leader.memory_footprint().unwrap();
        let total = tervy_term::kill::memory_footprints(&[session.owned_sessions()])[0];
        if members.len() >= 2 && total.is_some_and(|bytes| bytes > leader) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "session memory did not include its child"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut stale = session.owned;
    stale.leader.started_us += 1;
    assert!(stale.leader.memory_footprint().is_none());
    assert_eq!(
        tervy_term::kill::memory_footprints(&[vec![stale]]),
        vec![None]
    );
    session.signal(libc::SIGKILL).unwrap();
}

#[test]
fn final_output_is_drained_before_exit_and_grid_keeps_it() {
    let (session, events) = spawn("printf 'hello from PTY\\n'; exit 7");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut status = None;
    loop {
        while let Ok(event) = events.queries.try_recv() {
            session
                .handle
                .answer_query(&event, &Palette::default(), Dimensions::default());
        }
        while let Ok(event) = events.signals.try_recv() {
            match event {
                TerminalSignal::ChildExit(exit) => status = exit.code(),
                TerminalSignal::Exit => {
                    let snapshot = session.handle.snapshot(&Palette::default());
                    assert!(
                        snapshot
                            .runs
                            .iter()
                            .any(|run| run.text.contains("hello from PTY"))
                    );
                    assert_eq!(status, Some(7));
                    return;
                }
                TerminalSignal::Wakeup => {
                    let _ = session.handle.snapshot(&Palette::default());
                }
            }
        }
        assert!(Instant::now() < deadline, "PTY did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn child_has_default_sigint_and_receives_group_interrupt() {
    let (session, events) = spawn("trap -p; /bin/kill -l; printf '\\nREADY\\n'; sleep 30");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text: String = session
            .handle
            .snapshot(&Palette::default())
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        if text.contains("READY") {
            assert!(
                !text.contains("trap --"),
                "child inherited a signal handler: {text}"
            );
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    // SAFETY: owned, live child is a setsid process-group leader.
    assert_eq!(unsafe { libc::killpg(session.pid as i32, libc::SIGINT) }, 0);
    loop {
        if let Ok(TerminalSignal::ChildExit(status)) = events.signals.try_recv() {
            use std::os::unix::process::ExitStatusExt;
            assert!(status.signal() == Some(libc::SIGINT) || status.code() == Some(130));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "SIGINT did not terminate the child"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn direct_child_inherits_default_unblocked_signals() {
    let binary = std::env::temp_dir().join(format!("tervy-signals-{}", std::process::id()));
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/signals.c");
    assert!(
        std::process::Command::new("cc")
            .arg(source)
            .arg("-o")
            .arg(&binary)
            .status()
            .unwrap()
            .success()
    );
    let (session, events) = TerminalSession::spawn(
        SpawnSpec {
            program: binary.to_string_lossy().into_owned(),
            args: vec![],
            cwd: std::env::temp_dir(),
            env: HashMap::new(),
            scrollback_lines: 100,
        },
        Dimensions::default(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(TerminalSignal::Exit) = events.signals.try_recv() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let text: String = session
        .handle
        .snapshot(&Palette::default())
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    for signal in [
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGQUIT,
        libc::SIGTERM,
        libc::SIGALRM,
        libc::SIGPIPE,
        libc::SIGCHLD,
    ] {
        assert!(
            text.contains(&format!("SIGNAL {signal} default=1 blocked=0")),
            "{text}"
        );
    }
    std::fs::remove_file(binary).unwrap();
}

#[test]
fn terminal_queries_reply_without_rendering_or_a_ui_frame() {
    let script = r#"import os, tty, select, time

tty.setraw(0)
queries = [(b'\x1b[6n', b'\x1b[1;1R'), (b'\x1b[c', b'\x1b[?6c'),
           (b'\x1b[?u', b'\x1b[?0u')]
for query, expected in queries:
    os.write(1, query)
    readable, _, _ = select.select([0], [], [], .2)
    if not readable: raise SystemExit(11)
    reply = os.read(0, 512)
    if reply != expected: raise SystemExit(12)
os.write(1, b'PROBES_OK')
"#;
    let (session, events) = TerminalSession::spawn(
        SpawnSpec {
            program: "/usr/bin/python3".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: HashMap::new(),
            scrollback_lines: 100,
        },
        Dimensions::default(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut code = None;
    loop {
        match events.signals.try_recv() {
            Ok(TerminalSignal::ChildExit(status)) => code = status.code(),
            Ok(TerminalSignal::Exit) => break,
            _ => {}
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(code, Some(0));
    assert!(
        session
            .handle
            .snapshot(&Palette::default())
            .runs
            .iter()
            .any(|run| run.text.contains("PROBES_OK"))
    );
}

#[test]
fn concurrent_pty_allocations_all_start_and_teardown_cleanly() {
    // Parallel openpty calls reproduced -6 allocation failures in the M7
    // fixture. A barrier exercises the real serialized creation path.
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(20));
    let workers: Vec<_> = (0..20)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                TerminalSession::spawn(
                    SpawnSpec {
                        program: "/usr/bin/python3".into(),
                        args: vec![
                            "-u".into(),
                            "-c".into(),
                            "import os,signal; os.write(1,b'READY\\r\\n'); signal.pause()".into(),
                        ],
                        cwd: std::env::temp_dir(),
                        env: HashMap::new(),
                        scrollback_lines: 100,
                    },
                    Dimensions::default(),
                )
                .unwrap()
                .0
            })
        })
        .collect();
    let sessions: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    let deadline = Instant::now() + Duration::from_secs(10);
    for session in &sessions {
        while !session
            .handle
            .snapshot(&Palette::default())
            .runs
            .iter()
            .any(|run| run.text.contains("READY"))
        {
            assert!(
                Instant::now() < deadline,
                "launcher never reached its ready marker"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let owned: Vec<_> = sessions.iter().map(|session| session.owned).collect();
    drop(sessions);
    while owned
        .iter()
        .any(|session| !session.members().unwrap().is_empty())
    {
        assert!(
            Instant::now() < deadline,
            "reader teardown left a launcher alive"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
