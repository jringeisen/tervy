use devdesk_core::ipc::Server;
use std::{
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[test]
fn helper_projects_large_payload_without_gui_and_always_exits_zero() {
    let folder = std::env::temp_dir().join(format!("devdesk-helper-{}", std::process::id()));
    let server = Server::bind(&folder, || {}).unwrap();
    let id = "10000000-0000-4000-8000-000000000001";
    let mut child = Command::new(env!("CARGO_BIN_EXE_devdesk"))
        .args(["hook", "claude"])
        .env("DEVDESK_AGENT_ID", id)
        .env("DEVDESK_LAUNCH_ID", "7")
        .env("DEVDESK_HOOK_TOKEN", id)
        .env("DEVDESK_HOOK_SOCKET", &server.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let payload = format!(
        "{{\"last_assistant_message\":\"{}\",\"hook_event_name\":\"Stop\",\"session_id\":\"{id}\"}}",
        "x".repeat(200_000)
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let event = server.events.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(event.session_id.as_deref(), Some(id));
    assert_eq!(event.event, "Stop");
    assert_eq!(event.launch, 7);
    // Held-open stdin cannot keep the agent's hook waiting indefinitely.
    let start = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_devdesk"))
        .args(["hook", "claude"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    drop(stdin);
    assert!(start.elapsed() < Duration::from_secs(1));
    let output = Command::new(env!("CARGO_BIN_EXE_devdesk"))
        .args(["hook", "codex-notify", "not JSON"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    drop(server);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn codex_notify_chains_original_payload_even_without_ipc() {
    let folder = std::env::temp_dir().join(format!("devdesk-notify-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let callback = folder.join("callback.py");
    let result = folder.join("payload.json");
    std::fs::write(
        &callback,
        "import pathlib,sys\npathlib.Path(sys.argv[1]).write_text(sys.argv[-1])\n",
    )
    .unwrap();
    let payload = r#"{ "type":"agent-turn-complete", "thread-id":"10000000-0000-4000-8000-000000000001", "last-assistant-message":"synthetic fixture" }"#;
    let argv = format!(
        "[\"/usr/bin/python3\",\"{}\",\"{}\"]",
        callback.display(),
        result.display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_devdesk"))
        .args(["hook", "codex-notify", payload])
        .env("DEVDESK_CODEX_NOTIFY", argv)
        .env_remove("DEVDESK_HOOK_SOCKET")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    let deadline = Instant::now() + Duration::from_secs(3);
    while std::fs::read_to_string(&result).ok().as_deref() != Some(payload) {
        assert!(
            Instant::now() < deadline,
            "original notify did not receive unchanged payload"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::remove_dir_all(folder).unwrap();
}
