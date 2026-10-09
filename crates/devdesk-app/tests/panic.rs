//! Exercise the installed panic hook and real supervisor unwinding in a child.
use devdesk_term::kill::{OwnedSession, ProcessIdentity};
#[test]
fn panic_logs_location_without_payload_and_stops_children() {
    let folder = std::env::temp_dir().join(format!("devdesk-panic-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let config = folder.join("config.toml");
    std::fs::write(folder.join("worker.py"), "import os, pathlib, time\nif os.fork()==0:\n pathlib.Path('ready').touch()\n time.sleep(120)\nelse: time.sleep(120)\n").unwrap();
    std::fs::write(&config, format!("[[projects]]\nid='p'\nname='P'\npath={:?}\n[[projects.processes]]\nid='worker'\nname='Worker'\ncommand='/usr/bin/python3 -u worker.py'\nstop_timeout_secs=1\n", folder.to_string_lossy())).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_devdesk"))
        .arg("--panic-probe")
        .arg(&config)
        .env("HOME", &folder)
        .env_remove("DEVDESK_PERF_LAYOUT")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("PRIVATE_PROBE_PAYLOAD"));
    let identity = stderr
        .lines()
        .find_map(|line| line.strip_prefix("panic-probe sid="))
        .unwrap();
    let (pid, birth) = identity.split_once(" started_us=").unwrap();
    let owned = OwnedSession {
        leader: ProcessIdentity {
            pid: pid.parse().unwrap(),
            started_us: birth.parse().unwrap(),
        },
    };
    assert!(
        owned.members().unwrap().is_empty(),
        "panic left a managed child"
    );
    let log = std::fs::read_to_string(folder.join("Library/Logs/DevDesk/panic.log")).unwrap();
    assert!(log.contains("main.rs:"));
    assert!(!log.contains("PRIVATE_PROBE_PAYLOAD"));
    std::fs::remove_dir_all(folder).unwrap();
}
