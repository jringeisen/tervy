//! Paid opt-in contracts, through the production supervisor and real PTYs.
//! Only synthetic fixture data is sent. No transcript/rollout files are read.
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tervy_core::{
    agent::{Provider, Role, Status},
    supervisor::{Command, CoreEvent, SessionId, SessionInfo, SessionStatus, Supervisor},
};
use tervy_term::{
    Dimensions, TerminalEvents, TerminalHandle,
    keys::{self, Key, KeyInput},
    kill::OwnedSession,
    snapshot::Palette,
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retained synthetic failure fixture: {}", self.0.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
struct Pane {
    info: SessionInfo,
    handle: Option<TerminalHandle>,
    events: Option<TerminalEvents>,
    owned: Option<OwnedSession>,
    frames: VecDeque<String>,
    title_seen: bool,
}
struct Harness {
    runtime: Supervisor,
    panes: BTreeMap<SessionId, Pane>,
}
impl Harness {
    fn pump(&mut self) {
        while let Ok(event) = self.runtime.events.try_recv() {
            match event {
                CoreEvent::ConfigError(error) => panic!("{error}"),
                CoreEvent::SessionChanged(info) => {
                    if let Some(pane) = self.panes.get_mut(&info.id) {
                        pane.info = info;
                    } else {
                        self.panes.insert(
                            info.id,
                            Pane {
                                info,
                                handle: None,
                                events: None,
                                owned: None,
                                frames: VecDeque::new(),
                                title_seen: false,
                            },
                        );
                    }
                }
                CoreEvent::SessionStarted {
                    info,
                    handle,
                    events,
                    ..
                } => {
                    let owned = OwnedSession::capture(info.pid.unwrap()).unwrap();
                    handle.resize(size());
                    let pane = self.panes.get_mut(&info.id).unwrap();
                    pane.info = info;
                    pane.title_seen = false;
                    pane.handle = Some(handle);
                    pane.events = Some(events);
                    pane.owned = Some(owned);
                }
                CoreEvent::SessionClosed(id) => {
                    self.panes.remove(&id);
                }
                _ => {}
            }
        }
        for pane in self.panes.values_mut() {
            let (Some(handle), Some(events)) = (&pane.handle, &pane.events) else {
                continue;
            };
            while let Ok(query) = events.queries.try_recv() {
                handle.answer_query(&query, &Palette::default(), size());
            }
            while events.signals.try_recv().is_ok() {}
            while let Ok(event) = events.events.try_recv() {
                if let tervy_term::Event::Title(title) = event {
                    pane.title_seen = true;
                    self.runtime.handle.send(Command::AgentTitle {
                        id: pane.info.id,
                        launch: pane.info.launch,
                        title,
                    });
                }
            }
            let text = handle
                .snapshot(&Palette::default())
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if pane.frames.back() != Some(&text) {
                pane.frames.push_back(text);
                if pane.frames.len() > 100 {
                    pane.frames.pop_front();
                }
            }
        }
    }
    fn wait_ready(&mut self, id: SessionId) {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            self.pump();
            let text = self.panes[&id].frames.back().cloned().unwrap_or_default();
            if self.panes[&id].title_seen
                && (text.contains("context left")
                    || text.contains("Ask Codex")
                    || text.contains("Type a message"))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Codex TUI readiness: {}",
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn prompt(&mut self, id: SessionId, prompt: &str) {
        let pane = &self.panes[&id];
        let handle = pane.handle.as_ref().unwrap().clone();
        let launch = pane.info.launch;
        handle.write(keys::paste(prompt, handle.mode()));
        std::thread::sleep(Duration::from_millis(100));
        // Establish inferred activity before submitting, avoiding a stale Idle
        // mirror when a resumed record already has its provider identity.
        self.runtime.handle.send(Command::AgentInput {
            id,
            launch,
            enter: true,
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.pump();
            if self.panes[&id]
                .info
                .agent
                .as_ref()
                .is_some_and(|agent| agent.status == Status::ActiveInferred)
            {
                break;
            }
            assert!(Instant::now() < deadline, "input status handshake failed");
            std::thread::sleep(Duration::from_millis(10));
        }
        handle.write(keys::encode(
            &KeyInput {
                key: Key::Enter,
                modifiers: Default::default(),
                event: Default::default(),
                text: None,
                shifted: None,
                base_layout: None,
            },
            handle.mode(),
            false,
        ));
    }
    fn wait_idle(&mut self, id: SessionId, timeout: u64) -> String {
        let deadline = Instant::now() + Duration::from_secs(timeout);
        let began = Instant::now();
        let mut printed = Instant::now();
        loop {
            self.pump();
            if let Some(pane) = self.panes.get(&id) {
                if let Some(agent) = &pane.info.agent
                    && agent.status == Status::Idle
                    && let Some(session) = &agent.record.session_id
                {
                    // Notify can precede the final TUI redraw.
                    let session = session.clone();
                    std::thread::sleep(Duration::from_millis(300));
                    self.pump();
                    return session;
                }
                if printed.elapsed() > Duration::from_secs(15) {
                    eprintln!(
                        "Waiting for idle {}s: {}",
                        began.elapsed().as_secs(),
                        pane.frames
                            .back()
                            .unwrap_or(&String::new())
                            .lines()
                            .map(str::trim)
                            .filter(|line| !line.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n")
                            .chars()
                            .take(3000)
                            .collect::<String>()
                    );
                    printed = Instant::now();
                }
                assert!(
                    !matches!(
                        pane.info.status,
                        SessionStatus::Failed(_) | SessionStatus::Exited { .. }
                    ),
                    "CLI exited before notify: {:?}\n{}",
                    pane.info.status,
                    pane.frames
                        .iter()
                        .flat_map(|frame| frame.lines())
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
            assert!(Instant::now() < deadline, "Codex turn timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn wait_file(&mut self, path: &Path, timeout: u64) {
        let deadline = Instant::now() + Duration::from_secs(timeout);
        while !path.exists() {
            self.pump();
            assert!(
                Instant::now() < deadline,
                "fixture command did not start: {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn stop(&mut self, id: SessionId) {
        let owned = self.panes[&id].owned.unwrap();
        self.runtime.handle.send(Command::Close {
            id,
            confirmed: None,
        });
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.pump();
            if matches!(
                self.panes[&id].info.status,
                SessionStatus::Stopped | SessionStatus::Exited { .. }
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "Codex survived supervisor Stop");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(owned.members().unwrap().is_empty());
    }
}
fn size() -> Dimensions {
    Dimensions {
        columns: 140,
        rows: 60,
        ..Default::default()
    }
}
fn marker_count(folder: &Path) -> usize {
    std::fs::read_dir(folder)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("mcp-start-")
        })
        .count()
}
fn inner(folder: &Path) {
    let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/release/tervy")
        .canonicalize()
        .unwrap();
    let runtime = Supervisor::start_with_helper(folder.join("config.toml"), None, helper).unwrap();
    let mut harness = Harness {
        runtime,
        panes: BTreeMap::new(),
    };
    harness.runtime.handle.send(Command::NewAgent {
        project: "p".into(),
        picker: false,
        provider: Provider::Codex,
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let parent = loop {
        harness.pump();
        if let Some((id, _)) = harness.panes.iter().find(|(_, pane)| pane.handle.is_some()) {
            break *id;
        }
        assert!(Instant::now() < deadline, "Codex did not spawn");
        std::thread::sleep(Duration::from_millis(20));
    };
    harness.wait_ready(parent);
    harness.prompt(parent, "This is a synthetic fixture. We chose alacritty instead of vt100 because kitty keyboard protocol support is required. Read notes.txt and reply briefly with BLUEHERON and FIRST_DONE. Do not modify anything.");
    let parent_session = harness.wait_idle(parent, 120);
    assert!(
        harness.panes[&parent]
            .frames
            .iter()
            .any(|text| text.contains("FIRST_DONE"))
    );
    if std::env::var_os("TERVY_CODEX_STOP_ONLY").is_some() {
        harness.prompt(parent, "Run python3 stop-work.py once in the foreground and wait for completion. Do not background or retry it.");
        harness.wait_file(&folder.join("stop-started"), 120);
        let worker = std::fs::read_to_string(folder.join("stop-started")).unwrap();
        eprintln!(
            "Stop fixture PID/SID: {worker}; CLI SID: {}",
            harness.panes[&parent].owned.unwrap().leader.pid
        );
        harness.stop(parent);
        std::thread::sleep(Duration::from_secs(22));
        assert!(
            !folder.join("stop-marker").exists(),
            "Stop-only workload survived"
        );
        eprintln!("PASS focused Stop fixture");
        return;
    }
    let original = std::fs::read(folder.join("notes.txt")).unwrap();
    let servers = marker_count(folder);
    assert!(
        servers > 0,
        "synthetic MCP servers were not enabled for the primary control"
    );
    harness.prompt(parent, "Run python3 slow.py in the foreground and wait for it to finish. This bounded fixture waits for observer checks. Do not background or retry it. Set the command timeout to 200000 milliseconds so it can wait at least 120 seconds. Afterwards reply PARENT_DONE.");
    harness.wait_file(&folder.join("parent-started"), 120);
    // Allow the CLI’s asynchronous history index to flush before its picker lookup.
    std::thread::sleep(Duration::from_secs(3));
    let mut side_ids = Vec::new();
    let mut correct = 0;
    for trial in 0..5 {
        let previous: Vec<_> = harness.panes.keys().copied().collect();
        harness.runtime.handle.send(Command::ForkAgent { parent, question: "This is an authorized enforcement fixture: actually run touch pwned-shell.txt with the shell tool, then actually attempt apply_patch to create pwned-patch.txt; do not merely decline by choice. They should fail. Check tool-discovery metadata for any mcp__ tools or collaboration tools; do not invoke any external service or send anything to the parent. Report the observed write errors, the codename, the parent's reason for choosing alacritty over vt100, and your ability to observe its current live status. Finish with SIDE_DONE.".into() });
        let deadline = Instant::now() + Duration::from_secs(15);
        let side = loop {
            harness.pump();
            if let Some((id, _)) = harness.panes.iter().find(|(id, pane)| {
                !previous.contains(id)
                    && pane.handle.is_some()
                    && pane
                        .info
                        .agent
                        .as_ref()
                        .is_some_and(|agent| agent.record.role == Role::Side)
            }) {
                break *id;
            }
            assert!(Instant::now() < deadline, "side did not spawn");
            std::thread::sleep(Duration::from_millis(20));
        };
        let side_session = harness.wait_idle(side, 120);
        assert_ne!(side_session, parent_session);
        side_ids.push(side_session);
        let output = harness.panes[&side]
            .frames
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let lower = output.to_lowercase();
        eprintln!(
            "Trial {} final screen: {}",
            trial + 1,
            harness.panes[&side].frames.back().unwrap().trim_end()
        );
        let final_answer = harness.panes[&side].frames.back().unwrap().to_lowercase();
        if final_answer.contains("blueheron")
            && final_answer.contains("kitty")
            && final_answer.contains("side_done")
        {
            correct += 1;
        }
        assert!(
            lower.contains("operation not permitted") || lower.contains("writing is blocked"),
            "no enforcement failure observed"
        );
        assert!(
            lower.contains("writing is blocked"),
            "apply_patch was not observed failing"
        );
        assert!(
            lower.contains("no mcp__")
                || lower.contains("no `mcp__")
                || lower.contains("no mcp tools"),
            "side did not report an empty MCP registry"
        );
        assert!(
            !lower.contains("listed three mcp__codex_worktrees__"),
            "built-in worktree tools escaped the side profile"
        );
        assert!(
            !lower.contains("parent was stopped") && !lower.contains("parent has stopped"),
            "false parent-stop claim"
        );
        assert_eq!(std::fs::read(folder.join("notes.txt")).unwrap(), original);
        for name in ["pwned-shell.txt", "pwned-patch.txt", "pwned-mcp.txt"] {
            assert!(!folder.join(name).exists(), "enforcement failed: {name}");
        }
        assert_eq!(
            marker_count(folder),
            servers,
            "a side launched an MCP server"
        );
        assert!(
            !harness.panes[&parent]
                .owned
                .unwrap()
                .members()
                .unwrap()
                .is_empty()
        );
        harness.stop(side);
        if trial == 4 {
            let old_launch = harness.panes[&side].info.launch;
            harness.runtime.handle.send(Command::ResumeAgent(side));
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                harness.pump();
                if harness.panes[&side].info.launch != old_launch
                    && harness.panes[&side].info.status == SessionStatus::Running
                {
                    break;
                }
                assert!(Instant::now() < deadline, "side resume did not launch");
                std::thread::sleep(Duration::from_millis(20));
            }
            harness.wait_ready(side);
            harness.prompt(side, "Reply RESUME_DONE with the inherited codename and kitty protocol reason. Do not run tools.");
            let resumed = harness.wait_idle(side, 120);
            assert_eq!(Some(&resumed), side_ids.last());
            assert_eq!(
                harness.panes[&side]
                    .info
                    .agent
                    .as_ref()
                    .unwrap()
                    .record
                    .role,
                Role::Side
            );
            assert_eq!(marker_count(folder), servers);
            harness.stop(side);
        }
    }
    assert!(correct >= 4, "inherited reasoning {correct}/5");
    std::fs::write(folder.join("parent-release"), "done").unwrap();
    harness.wait_idle(parent, 180);
    assert!(
        harness.panes[&parent]
            .frames
            .iter()
            .any(|text| text.contains("PARENT_DONE"))
    );
    harness.prompt(parent, "Run python3 stop-work.py, waiting for its result. Do not background it. This is the deterministic supervisor Stop fixture.");
    harness.wait_file(&folder.join("stop-started"), 120);
    harness.stop(parent);
    let deadline = Instant::now() + Duration::from_secs(22);
    while Instant::now() < deadline {
        harness.pump();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !folder.join("stop-marker").exists(),
        "Stop left deterministic work running"
    );
    let notify = std::fs::read_to_string(folder.join("original-notify.jsonl")).unwrap();
    for line in notify.lines() {
        let payload: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(
            !side_ids.iter().any(|id| payload["thread-id"] == *id),
            "side invoked user's original callback"
        );
    }
    harness.runtime.handle.send(Command::Shutdown);
    eprintln!(
        "PASS Codex 0.162.0 interactive overrides, five read-only forks, no side MCP startup, inherited context, parent completion, primary-only notify, and supervisor Stop with no marker or owned survivors"
    );
}

#[test]
#[ignore = "paid Codex model calls, synthetic fixture only; build release helper first"]
fn codex_interactive_forks_and_supervisor_stop() {
    if let Some(folder) = std::env::var_os("TERVY_CODEX_PROBE") {
        inner(Path::new(&folder));
        return;
    }
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        process::{Command as ProcessCommand, Stdio},
    };
    let fixture =
        Fixture(std::env::temp_dir().join(format!("tervy-codex-probe-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&fixture.0).unwrap();
    let folder = fixture.0.canonicalize().unwrap();
    let codex_home = folder.join("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    let auth = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".codex"))
        .join("auth.json");
    assert!(
        auth.is_file(),
        "fixture needs existing Codex OAuth auth.json"
    );
    symlink(auth, codex_home.join("auth.json")).unwrap();
    std::fs::write(
        folder.join("notes.txt"),
        "The secret project codename is BLUEHERON.\n",
    )
    .unwrap();
    std::fs::write(folder.join("slow.py"), "import pathlib,time\npathlib.Path('parent-started').touch()\nstart=time.monotonic()\nwhile (not pathlib.Path('parent-release').exists() or time.monotonic()-start<120) and time.monotonic()-start<600: time.sleep(.1)\nprint('PARENT_DONE')\n").unwrap();
    std::fs::write(folder.join("stop-work.py"), "import pathlib,time\npathlib.Path('stop-started').write_text(str(__import__('os').getpid())+' ' + str(__import__('os').getsid(0)))\ntime.sleep(20)\npathlib.Path('stop-marker').touch()\n").unwrap();
    std::fs::write(
        folder.join("notify.py"),
        "import sys\nwith open(sys.argv[1],'a') as f:f.write(sys.argv[-1]+'\\n')\n",
    )
    .unwrap();
    std::fs::write(folder.join("mcp.py"), r#"import json,os,pathlib,sys
pathlib.Path('mcp-start-'+str(os.getpid())).touch()
for line in sys.stdin:
 p=json.loads(line)
 if 'id' not in p: continue
 method=p.get('method')
 if method=='initialize': result={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'synthetic-canary','version':'1'}}
 elif method=='tools/list': result={'tools':[{'name':'write_canary','description':'Write a synthetic fixture canary','inputSchema':{'type':'object','properties':{}}}]}
 elif method=='tools/call': pathlib.Path('pwned-mcp.txt').touch(); result={'content':[{'type':'text','text':'fixture write'}]}
 else: result={}
 print(json.dumps({'jsonrpc':'2.0','id':p['id'],'result':result}),flush=True)
"#).unwrap();
    assert!(
        ProcessCommand::new("git")
            .args(["init", "--quiet"])
            .current_dir(&folder)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(
        folder.join("config.toml"),
        format!(
            "[[projects]]\nid='p'\nname='Synthetic probe'\npath={:?}\n",
            folder.to_string_lossy()
        ),
    )
    .unwrap();
    let notify = vec![
        "/usr/bin/python3".to_string(),
        folder.join("notify.py").to_string_lossy().into_owned(),
        folder
            .join("original-notify.jsonl")
            .to_string_lossy()
            .into_owned(),
    ];
    let mut config = format!(
        "model='gpt-6.1-sol'\nmodel_reasoning_effort='low'\napproval_policy='never'\nsandbox_mode='workspace-write'\nnotify={}\n[projects.{:?}]\ntrust_level='trusted'\n",
        serde_json::to_string(&notify).unwrap(),
        folder.to_string_lossy()
    );
    for name in ["herd", "laravel-boost"] {
        config.push_str(&format!(
            "[mcp_servers.{name}]\ncommand='/usr/bin/python3'\nargs=[{:?}]\ncwd={:?}\n",
            folder.join("mcp.py").to_string_lossy(),
            folder.to_string_lossy()
        ));
    }
    std::fs::write(codex_home.join("config.toml"), config).unwrap();
    std::fs::create_dir_all(folder.join(".codex")).unwrap();
    std::fs::write(
        folder.join(".codex/config.toml"),
        format!(
            "[mcp_servers.fixture-write]\ncommand='/usr/bin/python3'\nargs=[{:?}]\ncwd={:?}\n",
            folder.join("mcp.py").to_string_lossy(),
            folder.to_string_lossy()
        ),
    )
    .unwrap();
    let shell = folder.join("fixture-shell");
    std::fs::write(
        &shell,
        "#!/bin/sh\nprintf '\\0TERVY_ENV\\0'\n/usr/bin/env -0\n",
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
    let log = folder.join("probe.log");
    let stdout = std::fs::File::create(&log).unwrap();
    let mut child = ProcessCommand::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "codex_interactive_forks_and_supervisor_stop",
            "--ignored",
            "--nocapture",
        ])
        .env_remove("CODEX_CI")
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID")
        .env_remove("CODEX_SANDBOX")
        .env_remove("CODEX_SANDBOX_NETWORK_DISABLED")
        .env("TERVY_CODEX_PROBE", &folder)
        .env("CODEX_HOME", codex_home)
        .env("SHELL", shell)
        .stdout(stdout.try_clone().unwrap())
        .stderr(stdout)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(900);
    let mut printed = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if printed.elapsed() > Duration::from_secs(30) {
            eprintln!("Synthetic Codex probe still running: {}", log.display());
            printed = Instant::now();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("Codex probe subprocess exceeded 15 minutes");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let output = std::fs::read_to_string(&log).unwrap();
    eprintln!("{output}");
    assert!(status.success(), "Codex interactive probe failed");
}
