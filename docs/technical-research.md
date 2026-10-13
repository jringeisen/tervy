# Tervy — Technical Research

Research date: **2026-10-08**. Original research machine: Apple Silicon (arm64), macOS 27.0.1, Command Line Tools only (no full Xcode), Rust initially not installed. Agent CLIs tested there: **Claude Code 2.1.295**, **codex-cli 0.161.0**. Those probes describe the old machine; migration verification is recorded in §10b below.

## How to read this document

Every capability claim carries a marker:

| Marker | Meaning |
|---|---|
| ✅ | Verified from an official source, a crate registry, or source code (link given) |
| 🧪 | Verified **empirically on the machine identified by the record** (see [Appendix A](#appendix-a--empirical-verification-log) for the original machine and §10b for the replacement) |
| 📘 | Verified from local `--help` output of the installed CLI version |
| ❓ | Assumption or inference, not yet verified; it has a planned verification step |

Unverified claims also appear in [§13 Assumptions register](#13-assumptions-register) so they don't hide in prose. All risks are consolidated in [§12 Identified risks](#12-identified-risks).

---

## 1. Summary of findings

| Area | Preference in brief | Finding | Recommendation |
|---|---|---|---|
| UI | GPUI | Viable, but the official crate is stale. Current GPUI is only available as a Zed git rev or a third-party snapshot | **GPUI via `gpui-component` 0.7.x, which pins `gpui-pre` exactly** (Longbridge weekly snapshot) |
| Terminal emulation | alacritty_terminal | Best available permissive option. It includes its own PTY and event loop | **`alacritty_terminal` =0.26.0, using its `tty` and `EventLoop`** |
| PTY | portable-pty | Redundant with alacritty's `tty`. Last crates.io release was Feb 2025 | **Drop it** |
| Async runtime | Tokio | GPUI has its own executors. Zed confines Tokio to a small bridge crate | **No Tokio.** Use a synchronous core with threads and channels. Use a `gpui_tokio`-style bridge only if a Tokio-only crate becomes unavoidable |
| Persistence | SQLite | `rusqlite` (sync, `bundled`) is simplest. `sqlx` needs Tokio. Zed's `sqlez` is GPL | **`rusqlite` 0.40 `bundled`** |
| Config | TOML or YAML | TOML is the Rust ecosystem default and supports comment-preserving edits | **TOML (`toml` + `toml_edit`)** |
| Claude Code | — | Session IDs, forks, hooks and tool restriction all work as needed (🧪) | Embedded TUI, with hooks injected through `--settings` |
| Codex | — | Fork and read-only sandbox work (🧪). Uses a shared daemon by default. Injected hooks are untrusted | Embedded TUI with `--no-daemon`, using `notify` and BEL for status |

---

## 2. Rust UI framework evaluation

The brief rules out Electron and web front ends. The realistic native-Rust options:

| Framework | Rendering | Fit for Tervy | Notes |
|---|---|---|---|
| **GPUI** (Zed) | GPU (Metal on macOS) | **Best.** Built for exactly this workload: a text-dense UI with a terminal (Zed's own terminal) | Pre-1.0, frequent breaking changes, sparse docs |
| egui | Immediate mode, GPU | Acceptable | Redraws every frame while animating. Harder to reach near-zero idle CPU and native feel; text shaping is less sophisticated |
| iced | Retained mode, wgpu | Possible | No reference terminal implementation of Zed's quality; more custom work for a fast grid |
| Slint | Retained, declarative DSL | Poor fit | DSL plus a licensing model (GPL, royalty-free or commercial) that adds friction for a later commercial product |
| Tauri / Dioxus (webview) | Web | Excluded by the brief | — |

**Verdict:** GPUI. It is the only Rust framework with a production-proven, high-throughput terminal built on it (Zed), and its event model (re-render only on `cx.notify()`) suits the near-zero idle CPU target.

## 3. GPUI maturity assessment

### Distribution (the most important finding)

- ✅ The official `gpui` on crates.io is **0.2.2 (2025-10-22)** and has not been updated since. — https://crates.io/crates/gpui
- ✅ Zed `main` has split GPUI into `gpui`, `gpui_platform`, `gpui_macos`, `gpui_apple` (Metal), `gpui_wgpu` and others. The README tells you to depend on `gpui` + `gpui_platform`, but **`gpui_platform` is not on crates.io**. — https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md
- ✅ There are three ways to get current GPUI:
  1. A git dependency on `zed-industries/zed` pinned to a `rev`.
  2. **`gpui-pre` 0.3.8** (2026-10-05): weekly snapshots of Zed `main`, published by **Longbridge (huacnlee), not Zed**, Apache-2.0. — https://crates.io/crates/gpui-pre
  3. `gpui-unofficial` (individual publisher). — https://crates.io/crates/gpui-unofficial
- ✅ README: "still pre-1.0. There will often be breaking changes between versions."

### Component library

- ✅ **`gpui-component` 0.7.1** (2026-10-05), in the `longbridge/gpui-kit` repo, Apache-2.0, released weekly. It provides `input`, `list`, `virtual_list`, `command`, `searchable_list`, `select`, `tab`, `resizable`, `dock`, `theme`, `sidebar`, `menu`, `notification` and `title_bar`. — https://crates.io/crates/gpui-component, https://github.com/longbridge/gpui-kit/tree/main/crates/component/src
- ✅ It **pins `gpui-pre` with an exact `=` requirement** (enforced in CI). Choosing gpui-component therefore also chooses the GPUI source.
- **Why it matters:** Tervy needs a text input (command palette, rename), a filtered list (palette), tabs and a resizable split. GPUI ships only primitives. Building a text input with IME, selection and clipboard from scratch is a multi-week detour. gpui-component removes that work.

### Build toolchain

- ✅ By default `gpui_apple/build.rs` compiles Metal shaders with `xcrun metal` / `metallib`, which needs full Xcode. On macOS 26+, Zed's docs also require `xcodebuild -downloadComponent MetalToolchain`. — https://github.com/zed-industries/zed/blob/main/crates/gpui_apple/build.rs, https://github.com/zed-industries/zed/blob/main/docs/src/development/macos.md
- ✅ The **`runtime_shaders`** feature skips offline compilation and compiles shaders when the app starts. gpui-kit enables it by default.
- ❓ A build with only Command Line Tools (this machine) *should* work with `runtime_shaders`. Not tested. **M0 acceptance criterion**, with Xcode + MetalToolchain as the fallback.
- ❓ Compiling shaders at runtime adds to cold start. **Measured in M1** against the 500 ms budget. If it's significant, switch to offline shaders (install Xcode).

### Renderer

- ✅ macOS still renders through Metal (`gpui_apple`). Linux and web moved to wgpu in Feb 2026 (Zed PR #46758), which helps a future Linux port. — https://github.com/zed-industries/zed/pull/46758

### Maturity verdict

GPUI is production-grade as a renderer (Zed ships on it) and immature as a library: there are no stable releases, docs are mostly examples (https://gpui.rs, `crates/gpui/examples`), and the API churns weekly. **Mitigation:** pin exact versions, upgrade deliberately (not on every release), and keep GPUI confined to the `app` crate so churn never reaches core logic.

## 4. Terminal emulator evaluation

| Option | License | Status | PTY / event loop | Verdict |
|---|---|---|---|---|
| **`alacritty_terminal`** | Apache-2.0 ✅ | 0.26.0 (2026-04-06), active ✅ | Yes: `tty` + `EventLoop` ✅ | **Recommended** |
| `wezterm-term` / `termwiz` | MIT ✅ | wezterm-term not on crates.io; termwiz 0.23.3 (2025-03), infrequent releases ✅ | termwiz: partial | Git dependency only; less proven when embedded |
| `vt100` | MIT ✅ | 0.16.2 (2025-07) ✅ | No | Too limited for full TUIs (kitty keyboard, etc.) ❓ |
| `libghostty-vt` (Rust bindings) | MIT OR Apache-2.0 ✅ | 0.2.2 (2026-09-28) ✅ | No | Build needs **Zig 0.16** and downloads Ghostty source; API pre-1.0. **Revisit in 2027** |
| Zed `terminal` crate | **GPL-3.0-or-later** ✅ | — | — | **Cannot be used or copied.** Study the ideas only |

### alacritty_terminal details (✅ from source)

- `Term<T: EventListener>`: the VT state machine and grid, behind `Arc<FairMutex<Term>>`.
- `term::Config { scrolling_history, .. }`: default 10,000 lines. This is the **bound on scrollback memory**.
- `tty::new(&Options, WindowSize, id)` (Unix):
  - calls `setsid()` and sets the PTY as controlling terminal (`TIOCSCTTY`), so **child pid == process group id == session id**;
  - `Pty::child()` exposes the `std::process::Child`;
  - on drop it sends **`SIGHUP` to the child pid only** (not the group);
  - **on macOS, with no shell specified it launches via `/usr/bin/login -flp`**. Tervy must always pass an explicit program.
- `EventLoop::new(...).spawn()`: **one OS thread per PTY**, 1 MiB read buffer. Write, resize and shutdown go through `Notifier` / `Msg`. ✅ `EventLoop::new` takes an existing `Arc<FairMutex<Term>>` (so a restart can reattach to the same grid) and **consumes** the PTY. It emits `ChildExit` *before* draining remaining output, then `Exit` (https://docs.rs/alacritty_terminal/latest/alacritty_terminal/event_loop/struct.EventLoop.html).
- Events Tervy **must handle**: `Wakeup` (redraw), `PtyWrite(String)` (terminal replies to queries such as cursor position (CPR) and device attributes (DA), which must be written back to the PTY), `ColorRequest` (OSC 10/11 queries, which Tervy must answer), `Title`, `Bell`, `ClipboardStore/Load` (OSC 52), `ChildExit(ExitStatus)` (0.26+), `Exit`.
- ✅ **Breaking changes in minor versions are normal** (the CHANGELOG bolds them; 0.26 changed `ChildExit`, 0.25 replaced `hold`). Pin `=0.26.0`.
- ✅ Zed uses a **fork** (zed-industries/alacritty) that adds `SignalMask` handling. ❓ Rust's `std::process` resets SIGPIPE and the signal mask in children, so Tervy may not need this. **M1 test:** confirm children start with default signal dispositions.
- ✅ **Key encoding is not part of `alacritty_terminal`.** Translating key presses to bytes (including the **kitty keyboard protocol**, which both agent TUIs use for Shift+Enter) is front-end work. Permissive references:
  - the `alacritty` binary crate (**Apache-2.0** ✅, 0.17.0), `input/keyboard.rs`;
  - `gpui-terminal` (MIT OR Apache-2.0 ✅).

### Rendering technique (ideas from Zed, re-implemented, never copied)

✅ From Zed's `terminal.rs` / `terminal_element.rs` (GPL; read for ideas only):
- **Event batching:** handle the first event immediately, then collect for **4 ms** or until **>100 events** queue, and merge repeated `Wakeup`s.
- **Text runs:** neighbouring cells with the same style become one `BatchedTextRun` (one shaped line). Background colours merge into rectangles.
- Default scrollback 10k, maximum 100k.

✅ `gpui-terminal` (zortax, MIT/Apache, 0.1.0) is a small permissive GPUI + alacritty_terminal reference. It targets the stale `gpui 0.2.2`, so it **won't build alongside gpui-component**. Read it, don't depend on it. — https://github.com/zortax/gpui-terminal

## 5. PTY library evaluation

- ✅ `portable-pty` 0.9.0. Last crates.io release 2025-02-11; still gets commits in the wezterm repo. MIT. It also calls `setsid` + `TIOCSCTTY`, and its `kill()` sends SIGHUP to the pid only.
- Since `alacritty_terminal` already ships a PTY spawner and reader loop wired into its `Term`, **portable-pty adds a second layer with no gain.** Dropped.
- ✅ **Neither library kills the whole process tree.** Tervy must do it itself:
  - `killpg(pgid, SIG)`. This works because pgid == child pid after `setsid`.
  - For interactive shells, the shell's job control puts jobs in **other** process groups within the same session. Shutting down needs the foreground group (`tcgetpgrp` on the PTY), plus SIGHUP to the shell (which forwards it to its jobs), plus a final sweep of processes in that session id. ❓ The macOS session-member enumeration API (`proc_listallpids` + `proc_pidinfo`) is to be confirmed in M3.

## 6. Async runtime evaluation

- ✅ GPUI ships its own `ForegroundExecutor` (main thread) and `BackgroundExecutor` (thread pool).
- ✅ Zed uses Tokio **only** through `gpui_tokio` (Apache-2.0): a separate 2-worker runtime. Source comment: "Since we now have two executors, let's try to keep our footprint small". — https://github.com/zed-industries/zed/blob/main/crates/gpui_tokio/src/gpui_tokio.rs
- ✅ Zed uses `smol` / `async-process` for subprocesses.
- **What Tervy needs async for:**
  - PTY reading: alacritty's own thread.
  - Child exit: the alacritty `ChildExit` event.
  - Timers (stop grace periods, restart backoff): a channel-receive deadline.
  - Unix socket for hooks: a blocking accept thread.
  - SQLite: synchronous.
  - Config watching: the `notify` crate's thread.

  **None of these need Tokio.**
- **Recommendation:** the core is **synchronous Rust (threads + `async-channel`)**, and it runs no async runtime of its own. The UI awaits core events through `async-channel`'s async `recv()` on GPUI's foreground executor. Adding Tokio later is a contained change (a bridge like `gpui_tokio`).

## 7. SQLite integration options

| Crate | Model | Verdict |
|---|---|---|
| **`rusqlite` 0.40.2** (MIT ✅, 2026-08-08) | Synchronous, `bundled` compiles SQLite in | **Recommended**: simple, no runtime, ideal for small, infrequent writes from a core thread |
| `sqlx` 0.9.0 (MIT/Apache ✅) | Async, needs Tokio/async-std | Rejected: drags in Tokio for no benefit |
| `sqlez` (Zed) | Wrapper over libsqlite3-sys | **GPL**, not on crates.io. Rejected |

Migrations: a hand-rolled `PRAGMA user_version` + ordered SQL scripts (≈30 lines). The `rusqlite_migration` crate is an alternative. ❓ Not needed for an MVP with 3–4 tables.

**Is SQLite even needed?** It was challenged. App state is small (UI layout, agent session records, the session ledger). A JSON file would work at first, but agent session history grows without bound, needs per-project queries ("recent Claude sessions in Billi"), and must survive a crash mid-write. SQLite (WAL) gives atomicity and queries for ~1 MB of binary. **Kept.**

## 8. Claude Code integration research

Docs base: https://code.claude.com/docs/en/ (pages: `cli-reference`, `hooks`, `sessions`, `permissions`, `permission-modes`, `sandboxing`, `settings`, `env-vars`, `terminal-config`, `agent-view`, `agent-sdk/sessions`).

### Session identity and lifecycle
- 📘✅ `--session-id <uuid>` makes **Tervy choose the ID before launch**, so no discovery is needed.
- 📘✅ `--resume <id>` / `-r` resumes. With no ID it opens an interactive picker, which gives free access to sessions not started in Tervy.
- 📘✅🧪 `--fork-session` (with `--resume`) creates a new session ID from the resumed history.
- 🧪 **Forking a *running* session works.** The parent kept running and finished normally; the fork got a new ID and had the parent's context (Appendix A, test A2).
- 🧪 **Caveat:** the fork's snapshot showed the parent's **in-flight tool call as stopped**. The fork told the user the parent's `sleep` "was stopped before it finished", which was false.
- ✅ Transcripts are stored at `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`. The **path** is documented, but the **format** is documented as internal and subject to change. **Tervy will not parse transcripts.**
- ✅ **Resume does not restore** `--settings`, `--mcp-config`, `--add-dir` and similar flags. They must be passed again on every resume or fork.

### Hooks (status tracking)
- ✅ 33 hook events. Relevant ones: `SessionStart` (`source`: startup | resume | clear | compact | fork), `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, **`PermissionRequest`** (dedicated needs-approval signal), `Notification` (`notification_type`: `permission_prompt` ≈6 s, `idle_prompt` ≈60 s, `agent_needs_input`, …), `Stop`, `SessionEnd`.
- ✅ Common input fields: `session_id`, `transcript_path`, `cwd`, `hook_event_name`.
- 🧪 **Hooks passed as an inline JSON string via `--settings` run**, and every event carries the correct `session_id` (Appendix A, test A5).
- 🧪 A forked session's `SessionStart` fires with **`source: "fork"` and the new session ID**. This is how Tervy learns fork IDs (test A6).
- ✅ `--settings` overrides matching keys, and array settings merge. ❓ Whether `--settings` hooks *add to* the user's own `~/.claude/settings.json` hooks (expected) rather than replacing them: to be checked with a probe in M4.
- ✅ Handler types: `command`, `http`, `mcp_tool`, `prompt`, `agent`. Tervy uses `command`, pointing at its own helper, which writes to a Unix socket.

### Permission enforcement
- 📘✅ `--tools <list>` limits which **built-in** tools exist. It does **not** affect MCP tools.
- 📘✅ `--disallowedTools <name>` removes a tool; `"mcp__*"` removes all MCP tools. `--strict-mcp-config` loads only MCP servers given via `--mcp-config`.
- 📘✅ `--restricted` removes command-running tools and WebFetch unless they're named in `--tools`, confines file tools to the working directories, ignores user/project/local settings files (`--settings` still applies), and refuses `bypassPermissions`.
- 📘✅ `--permission-mode dontAsk`: anything that would prompt is denied.
- 🧪 With `--tools "Read,Grep,Glob"`, Edit, Write and Bash calls were **rejected by the harness**, and the file stayed unchanged (tests A3, A4).
- 🧪 **Without `--strict-mcp-config`, MCP connector tools stay available** to the fork (some can write to external services).
- 🧪 **The model's self-reported tool list is unreliable.** In one run it claimed Edit was available; it wasn't. Enforcement must be judged by trying the action, never by asking the model.
- ✅ Sandboxing (`sandbox.enabled`) covers **Bash only**. File tools, hooks and MCP run outside the sandbox, so a sandbox **alone** cannot make an agent read-only.
- ❓ All enforcement tests ran in `-p` (print) mode. The flags aren't documented as print-only, but **interactive-mode enforcement is unverified** and is the first acceptance criterion of the side-conversation milestone.

### Other relevant capabilities
- ✅ `claude --bg` / `claude agents` / `attach` / `claude daemon`: **Claude's own background-session supervisor** (research preview), with states such as working, blocked and done available as JSON. Not used in the MVP. It's a candidate building block if "agents survive app quit" is ever wanted (see open-questions).
- ✅ Official SDKs exist for TypeScript and Python only (session list/fork APIs exist there). **No Rust SDK.** The docs recommend running the CLI as a subprocess.
- ✅ Useful environment variables: `CLAUDE_CODE_NONBLOCKING_STDOUT` (a stalled terminal can't freeze Claude), `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`, `CLAUDE_CONFIG_DIR`.
- ✅ Shift+Enter works without setup in terminals with the kitty keyboard protocol (v2.1.269+). Without it, Ctrl+J works as a fallback.

## 9. Codex integration research

Docs: https://developers.openai.com/codex/* (now redirects to https://learn.chatgpt.com/docs/*). Source: https://github.com/openai/codex (`codex-rs/`), checked at HEAD on 2026-10-08.

### Session identity and lifecycle
- 📘 There is **no `--session-id` flag**, so the ID must be captured after launch.
- 📘 `codex resume [ID]` (picker if omitted), `codex fork [ID]`, `codex exec resume|fork <ID>`.
- 🧪 **Forking a running session works.** The parent finished normally and the fork got a new `thread_id` (test B1).
- ✅🧪 Mid-turn forks use `ForkSnapshot::Interrupted`: the fork's history gets a synthetic `<turn_aborted>` marker. In our test the fork **tried to poll the parent's running process** (`write_stdin failed: Unknown process id`). In separate processes this harmlessly fails.
- 📘 `codex exec fork` does **not** accept `-s`. Use `-c 'sandbox_mode="read-only"'`. The interactive `codex fork` accepts `-s`.
- ✅ Ways to capture the ID, best first:
  1. **legacy `notify` payload** (`thread-id`, sent at the end of each turn);
  2. `SessionStart` hook (needs hook trust; see below);
  3. the exit summary printed by the TUI (`codex resume <id>`);
  4. app-server `thread/*`.

  Rollout files (`~/.codex/sessions/...`) are **undocumented and migrating**, so avoid them.

### The shared daemon (critical)
- 📘 Feature `daemon_auto_start` is stable and on. The TUI attaches to a shared, detached **app-server daemon**.
- ✅ When the TUI exits it prints "Disconnected from this task. Any running work continues." **Killing the TUI does not stop the agent's turn.**
- ✅ `-s` and `-a` **do not** force in-process (embedded) mode. Most `-c` overrides and `--no-daemon` **do**.
- **Consequence:** under Tervy's stop-on-quit model, **every Codex invocation Tervy builds (start, resume, fork) must pass `--no-daemon`**. Otherwise "Stop" leaves work running in a daemon that's shared with the user's other Codex terminals. Never call `codex app-server daemon stop`.

### Status signals
- ✅ Hooks exist (SessionStart, UserPromptSubmit, PermissionRequest, Stop, Interrupt, SessionEnd, …), but non-managed hooks only run when their hash is **trusted**. Hooks injected through `-c` show as untrusted and are **skipped**. `--dangerously-bypass-hook-trust` trusts **all** non-managed hooks, including untrusted repo hooks. **Rejected as a default.**
- ✅ Legacy `notify`: `-c notify='["<helper>"]'` runs with **no trust check**. Its payload is `type: "agent-turn-complete"`, `thread-id`, `turn-id`, `cwd`, `last-assistant-message`. It is the **only** notify type (no approval event).
- ✅ TUI notifications: `tui.notifications` (`agent-turn-complete`, `approval-requested`, …), `tui.notification_method` (`osc9` | `bel` | `auto`), `tui.notification_condition` (`unfocused` | `always`).
- **Assembled design:** `notify` (turn done + thread ID) + `tui.notifications=["approval-requested"]` with `method="bel"`, `condition="always"`. alacritty_terminal emits `Event::Bell`, so in a Codex pane **BEL means needs-approval**. "Working" = the user pressed Enter in the pane (Tervy is the terminal, so it sees the input). ❓ Verify in M6.

### Sandbox
- ✅🧪 On macOS, `read-only` uses Seatbelt (`sandbox-exec`). Writes fail with `Operation not permitted` at the OS level (test B1). Network is off by default in read-only.
- 🧪 Read-only shell commands still run (the fork ran shell commands; only the write failed). So a **Codex side conversation can run `git diff`**, while a Claude one cannot. This asymmetry is accepted.
- 📘 `-a never` means unattended (failures go back to the model). Avoid `--approve-for-me`, which forces workspace-write.

### App-server protocol
- ✅ JSON-RPC over stdio, Unix socket or WebSocket. Offers `thread/start|resume|fork|list`, `turn/start|interrupt`, approval requests, and `thread/status/changed` (`idle`, `active` + `waitingOnApproval` | `waitingOnUserInput`). This is the **richest** status source.
- ✅ Labelled **experimental** ("not supported for production workloads"), although OpenAI's own TUI is an app-server client.
- **Deferred:** a later option is for Tervy to run its own `codex app-server --listen unix://…`, point the TUI at it with `--remote`, and read status from its own connection.

### Terminal requirements (✅ from codex-rs source)
- The startup probe sends CPR (`ESC[6n`), OSC 10/11 colour queries, `ESC[?u` and DA1, with a **250 ms timeout**. **An embedded terminal that doesn't answer makes every launch stall.**
- Kitty keyboard flags, bracketed paste, focus events (`?1004`), alternate scroll, mouse modes, OSC 8 hyperlinks.

## 10. Configuration and other crates

| Crate | Version | License | Use |
|---|---|---|---|
| `toml` / `toml_edit` | current | MIT/Apache | Parse config; `toml_edit` preserves the user's comments when Tervy adds a project |
| `serde` | current | MIT/Apache | (De)serialisation |
| `notify` | 8.2.0 ✅ | CC0-1.0 ✅ | Watch `config.toml` for hot reload |
| `async-channel` | current | MIT/Apache | Core→UI events (blocking send, async receive) |
| `vte` | 0.15.0 ✅ | MIT/Apache ✅ | Pulled in by alacritty_terminal |
| `libc` | current | MIT/Apache | `killpg`, `tcgetpgrp` |
| `uuid` | current | MIT/Apache | Claude session IDs |

YAML rejected: no stable Rust YAML stack (`serde_yaml` is unmaintained) and indentation-sensitive hand editing.

## 10a. M0 build record (2026-10-08)

The compiling manifest is the repo's root `Cargo.toml` (`gpui-kit = "=0.7.1"`, `alacritty_terminal = "=0.26.0"`, edition 2024, rustc 1.99.0). Platform setup is in `crates/tervy-app/src/main.rs`:
- `gpui_kit::application().with_assets(Assets)` → `on_reopen` → `run(init, menus/keys, open_window, activate)`.
- Imports go through the facade only: `gpui_kit::*`, `gpui_kit::component`, `gpui_kit::base::StyledExt`.
- `gpui_kit::actions!` replaces `gpui::actions!` (lib.rs says the upstream derive path doesn't resolve through the facade).
- `on_reopen` is an `Application` method, not an `App` method.
- On macOS, `QuitMode::Default` is `Explicit`, so closing the window doesn't quit (matches ADR-018).

**License audit (cargo-deny):** three crates outside the allowlist, all transitive through GPUI and allowed as narrow per-crate exceptions in `deny.toml`:
- `cbindgen` (MPL-2.0): build-time only, not shipped.
- `option-ext` (MPL-2.0): linked, via `dirs-sys` ← GPUI's font-kit. **Flagged for the commercial audit.**
- `libbz2-rs-sys` (bzip2-1.0.6): a permissive BSD-style license.

**Upstream warning:** `block` 0.1.6 (GPUI's Metal stack) "contains code that will be rejected by a future version of Rust". Not ours to fix; watch on gpui-kit upgrades.

## 10b. M0 migration verification (2026-10-08)

- 🧪 Replacement machine: MacBook Pro `Mac14,6`, Apple M2 Max (12 CPU cores), 96 GB RAM, macOS 27.0.1 (`26A434`). Xcode 27.0 (`27A266a`) was already installed and selected; no Xcode installation was needed. This build does not independently re-prove the original machine's CLT-only result.
- 🧪 Installed Rust stable via rustup: `rustc 1.99.0 (b940084d7 2026-09-28)`, and Homebrew `cargo-deny 0.20.2`. Homebrew's post-install cleanup reported a cache permission error; the installed `cargo deny --version` succeeds.
- 🧪 The transferred `target/` contained cached binaries from the old machine. Cargo reused a core test executable carrying `com.apple.quarantine` from `sharingd`; it stalled at `_dyld_start` (confirmed with `sample`). Removed generated artifacts with `cargo clean` to force local builds. Initial startup/idle runs using transferred binaries were discarded. A freshly built core test also stalled before `main`; its ad hoc signature verified, but `syspolicyd` logs explicitly reported Gatekeeper rejection. The workspace/source paths also carried AirDrop quarantine metadata. After explicit owner approval, removed only `com.apple.quarantine` from the workspace root, source/build paths and Cargo/config files (excluded `.git` and agent configuration). The complete `scripts/check.sh` then passed: formatting, clippy, unit/doc test executables (currently zero tests in the M0 scaffold), and license audit. When moving machines, transfer source and `Cargo.lock`, and rebuild `target/` locally.
- 🧪 Shell rc inspection found nvm and Herd setup in `~/.zshrc`, and Homebrew setup in `~/.zprofile`. With a clean environment (`HOME`, `USER`, `TMPDIR`, `TERM=xterm-256color`, `PATH=/usr/bin:/bin`), the command lookup probe returned:

  | zsh mode | npm | php |
  |---|---|---|
  | `-c` | absent | absent |
  | `-l -c` | `/usr/local/bin/npm` | `/opt/homebrew/bin/php` |
  | `-i -c` | nvm Node v22.21.1 | Herd |
  | `-l -i -c` | nvm Node v22.21.1 | Herd |

  🧪 `/etc/zprofile` still invokes `path_helper`. ADR-026's environment resolution remains appropriate; login-only resolves different tools here.
- ❓ Agent CLI versions and flag behavior have not been re-verified on the replacement machine. Re-run the applicable Appendix A probes before relying on their behavior in agent integration milestones. Codex MCP configuration enumeration remains an M6 task.
- 🧪 Locally rebuilt release baseline (`perf/baseline.json`): ten startup runs, p50 **169.9 ms**, p95 **473.2 ms** (first run **473.2 ms**, included in both percentiles). Startup meets the 500 ms p50 budget. Sequential launches do not flush macOS/Metal caches.
- 🧪 Empty release window after 10 s settle, with no managed children or build activity: **61 MB** footprint and **0.553%** mean CPU over 30 sample intervals (**33 s** wall time). Memory and CPU exceed the 60 MB / 0.3% budgets. `top -l 31 -s 1 -pid <pid> -stats cpu,idlew` reported a cumulative idle-wakeup counter from **7 to 33**: **26 / 33 s = 0.788 wakeups/s**. A separate `top -d` probe confirmed delta counting. The default `IDLEW` column must not be averaged as a rate. ❓ The old machine's reported ~76 wakeups/s cannot be validated without its raw samples, and process idle wakeups alone cannot attribute Tervy timers vs GPUI/OS activity. A11 investigation still starts M1; no performance fix has been made in M0.

## 10c. A11 idle investigation before terminal implementation (2026-10-08)

🧪 Three release controls on the replacement machine, each after 10 s settle and over 30 s, recorded in `perf/a11-empty-window.json`:

| Empty layout | CPU | Footprint (decimal MB) | Package-idle wakes/s | Interrupt wakes/s |
|---|---:|---:|---:|---:|
| Normal sidebar | 0.664% | 63.52 | 1.167 | 120.32 |
| Sidebar removed | 0.675% | 63.31 | 0.933 | 120.35 |
| Blank background only | 0.634% | 60.13 | 0.967 | 120.36 |

- 🧪 Stack samples of all controls show `CVDisplayLink::runIOThread`, GPUI's `display_link_output_callback` and `window::step`. Workspace render tracing stops after startup; no continuing content paints were observed. No terminal/cursor exists yet. Sidebar removal does not remove the recurring activity.
- ✅ Pinned registry source: `gpui-pre-macos-0.3.8/src/window.rs` (`start_display_link`, `display_layer`, `step`) and `src/display_link.rs` (`WindowFrameSource`, `subscribe`) keep a visible window subscribed to CoreVideo until occlusion or teardown. `gpui-pre-0.3.8/src/window.rs` avoids drawing when clean, but still receives frame callbacks. Its `PlatformWindow` interface has no stop/rearm API exposed to the app for this backend. The sidebar's timer is a finite hide transition, not a continuing idle loop (`gpui-component-0.7.1/src/sidebar/mod.rs`).
- ✅ Native `proc_pid_rusage` CPU fields are Mach ticks, not nanoseconds: [XNU fill_task_rusage](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/bsd_kern.c) takes values from [task_power_info_locked](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/task.c). The sampler converts with `mach_timebase_info`. 🧪 A single-core busy process validated the sampler at 100.3%. Package-idle wakeups and interrupt wakeups are different counters; the low package-idle number does not mean the display link is stopped.
- 🧪 Reproduce after a release build: `python3 scripts/perf/idle.py --layout minimal --seconds 30 --sample --output /tmp/tervy-idle.json`; repeat with `normal` and `no-sidebar`. The sampler does no recurring work inside Tervy. `TERVY_LOG=tervy=trace` records actual workspace renders in the diagnostics log.
- ❓ A11 remains an **upstream limitation**, not fixed: interrupt-wakeup rate is consistent with display pacing; exact refresh-rate scaling needs another display configuration to verify. App-level notification or blink changes cannot stop the backend's private frame source. A fix requires a reviewed upstream dependency patch providing demand-based stop/rearm, including input, resize, appearance, GPU recovery and animation regression coverage. No unsafe AppKit workaround, dependency fork or pin change is introduced here. CPU and memory remain over budget; the terminal will add no idle polling or blink timer.

## 10d. M1 terminal evidence on the replacement machine (2026-10-08)

- 🧪 `scripts/check.sh`: formatting, Clippy (`-D warnings`), 23 tests and license checks pass. Actual PTY tests verify final-output draining, exit status, default/unblocked child signals, foreground-group SIGINT and protocol replies without a UI frame. A deterministic interleaving test uses the real snapshot method to verify output cannot lose its wakeup. Tests replay only captured terminal protocol bytes, never transcripts or rollout files.
- 🧪 The cached ADR-026 resolver finds nvm Node/npm and Herd PHP on this machine. Interactive startup initially printed a `compinit` error from a dangling Homebrew `_brew_services` symlink pointing to the removed services tap. Reversibly hiding that link fixes fresh login-shell startup; it is a host repair, not a terminal parser change. Owner verified prompt/basic typing and the Option-e accent fix. Fractional font advance now anchors adjacent ASCII and accented cells without a drawn gap.
- 🧪 **A10:** `Cell` is 24 bytes, so 5,000 × 120 cells have a 14.4 MB inline floor. Actual headless Term/parser peak RSS includes both grids, allocator and metadata. Fresh release processes plateau near 19.1 MB for plain/styled output, 19.7 MB for distinct links, and 77.0 MB when every cell has eight combining marks (`perf/memory-probe.json`, samples at 5,000/10,000/20,000 lines). Thus 24 B/cell is a floor, not a worst-case estimate. GUI `seq 1 2000000` is measured separately in the baseline.
- 🧪 Fixed scrollback alone did **not** bound upstream metadata: the initial probe retained 100,000 combining marks in one cell and a 100 KB hyperlink. Attributed patches to the pinned Apache/MIT registry sources cap combining marks at eight per cell, link URI/ID at 8,192/256 bytes, live link allocations at 256 per terminal, title text at 200 characters, and OSC payload at 160 KiB. Excess metadata is discarded while normal text survives. Combining/link/OSC recovery tests pass. Exact changes and upgrade obligations: `vendor/README.md` and `vendor/tervy.patch`. No new license exception or dependency version change.
- 🧪 Startup controls separated UI layout from process startup: terminal prepaint took under 1 ms; a terminal view without shell spawning launched near the M0 median, while concurrent shell startup increased median first frame to roughly 450 ms in the standard harness. Deferring shell/env startup until after the first window frame removes that contention. The first-frame benchmark exits before spawning a shell; CLI content timing separately includes env resolution and the deferred start. Final first-frame p50 **164.1 ms**, p95/first post-build launch **502.4 ms**; all ten launches are in `perf/baseline-m1.json`. The 500 ms median budget passes; the first launch individually exceeds 500 ms. OS/Metal caches are **not** flushed; these are sequential process launches, not proof of true cold-cache performance.
- 🧪 The visible release probe dispatches real GPUI keystrokes through native input at varying frame phases. Baseline records both dispatch→PTY-write and dispatch→painted-echo samples; physical keyboard delivery and compositor scanout are excluded. A printable 100 decimal MB `cat` flood accepts queued input and continues painting. Two-million-line `seq` retains the 5,000-line history cap. Headless memory probes and external GUI footprint samples are distinct measurements.
- 🧪 Installed versions: Claude Code 2.1.295 and codex-cli **0.162.0**. Actual startup-query fixtures show prompt CPR/DA/kitty/color replies; direct PTY tests enforce a 200 ms reply deadline without rendering. CLI probes submit **no model prompt**. Baseline reports view-init→first nonblank content and PTY-spawn→first nonblank content separately; neither proves authentication/readiness or every interactive screen. Agent process footprints are sampled separately from Tervy. ❓ Full fork/read-only Appendix A probes must be repeated before M4–M6 rely on changed-version flags.
- 🧪 Final isolated idle shell after 10 s settle, over 60 s: Tervy **65.49 MB**, **0.758% CPU**, **0.133 package-idle wakes/s**, **120.50 interrupt wakes/s**. Its separate shell child used **4.83 MB** and no measured CPU. The one-terminal memory budget passes; the **0.3% CPU budget is missed**. Echo sample p95 is **9.21 ms**, PTY-write sample p99 **1.534 ms** (20 synthetic events, not a population guarantee). The 100 MB flood completes with queued input preserved; `seq 1 2000000` plateaus near **112 MB** with full scrollback. Claude/Codex PTY-spawn→first content took **280.4/59.6 ms**, and view-init→content **843.2/636.7 ms**.
- 🧪 Owner approved the running M1 window on 2026-10-08 (“looks good lets continue”); the individual TUI/clipboard/IME/Unicode checklist is retained in `docs/m1-review.md`. Pinned GPUI omits physical keypad/base-layout event metadata, limiting those adapter distinctions despite tested encoders. A11 remains the upstream display-link limitation in §10c; idle CPU varies with window/display conditions. M2 lifecycle guarantees and M7 multi-session budgets are not claimed by M1.

## 10e. M2 lifecycle and workspace evidence (2026-10-08)

- 🧪 `scripts/check.sh` passes with **32 tests**, including ten busy real shell sessions, a stop requested before spawn finishes, foreground-close confirmation for the current launch, config reload and single-instance activation. Core stays synchronous and GPUI-free. Output and keystrokes bypass the supervisor; only child lifecycle events enter its mailbox. The idle actor waits without a timer.
- 🧪 The selected macOS SDK exposes `proc_listallpids`, `proc_pidinfo(PROC_PIDTBSDINFO)`, `getsid`, `tcgetpgrp` and `killpg` through libc. Tests cover two background process groups, jobs ignoring SIGHUP/SIGTERM, wrong birth-time identity, and a live child after its leader exits. Sweeps exclude zombies and recheck birth time before signalling; macOS provides no atomic identity-and-kill operation. A self-`setsid` daemon remains outside scope. Parallel shutdown escalates at 0/2/5 seconds and has a 15-second global deadline; an unverified cleanup reports `clean=false`. Crash-relaunch ledger recovery remains M3.
- 🧪 The parent-directory config watcher detects hand edits and atomic replacements with 120 ms debounce. The real watcher test updates within one second; invalid TOML retains the last good config, including for newly opened shells. Canonicalization fixes FSEvents' `/private/var` versus `/var` path mismatch. Add Project uses `toml_edit`, re-reads at mutation time, refuses invalid current contents and atomically renames a synced temporary file. Its pre-rename concurrent-edit check is best-effort, not an atomic compare-and-swap against external editors.
- 🧪 Pinned gpui-component Input/Command/TabBar support rename, palette and tabs. Command's built-in filter is substring-based; a plain case-insensitive subsequence matcher supplies fuzzy filtering. The release stress probe opens the palette and finds all three project commands for `swpj`. Production split controls remain M5; stress instrumentation alone displays three terminal entities. The initial benchmark dispatched a key while holding a Workspace entity lease and triggered GPUI reentrant painting; releasing the lease before dispatch fixes that harness panic. A later timed-out trial has no confirmed cause; the final bounded probe completes and independently verifies every owned session is empty.
- 🧪 Final release measurements and comparison with M1 are in `perf/baseline.json`; M1 is preserved in `perf/baseline-m1.json`. The probe uses **ten `yes` producers plus one quiet raw echo child**, three projects and three visible panes. Twenty synthetic keys and ten switches exercise the real production supervisor and terminal renderer. An external 30-second bound and identity-checked cleanup guard against orphaned noisy children. No agent or model prompt is involved. Frame callbacks exclude compositor scanout and physical keyboard delivery; limited samples are not population tail guarantees.

- 🧪 M2 final numbers: first-frame p50 **164.3 ms**, first/p95 **619.4 ms** (402.8 ms before main); ten noisy terminals frame p95 **9.30 ms**, max switch **10.90 ms**, quiet echo p95 **8.94 ms**, palette **8.29 ms**, shutdown **108.08 ms**, all eleven sessions empty. One stable idle shell uses **70.06 MB / 0.0073% CPU / 0.383 interrupt wakes/s** over 60 seconds after a 10-second settle. Visibility/occlusion is uncontrolled; low CPU in this trial is not an A11 fix. The earlier 84.85 MB / 1.50% trial logged five shell launches and is retained as nonbaseline evidence. Details and regression limits are in the M2 review.

## 10f. M3 process lifecycle and recovery evidence (2026-10-08)

- 🧪 `core::process` owns policy state and monotonic retry times; the supervisor owns PTYs and signals. Configured launches use the cached environment and non-login `$SHELL -c`. Stop escalation uses the current instance's immutable config snapshot. No timer is armed solely for a stopped process; recovery suspends retry scheduling.
- 🧪 A crashed alternate-screen/mouse/kitty fixture restarts on the same `Arc<Term>`, retains primary and crash text, restores default modes, and answers a cursor query through the new instance's input channel. Direct Handler calls implement ADR-027 without RIS/DECSTR. ✅ Rebinding the pinned Term's private event sink required one attributed public setter in the local Apache-2.0 source; see `vendor/tervy.patch`.
- 🧪 Real-PTY integration tests verify leaderless port-holder cleanup before rebinding, history across automatic retries, Stop cancellation, first-open autostart and unchanged running config. A separate supervisor test process is SIGKILLed; relaunch recovers and kills its ledger-tracked surviving child. Stale birth-time records are discarded; existing identity tests reject reused processes.
- 🧪 Bundled rusqlite 0.40.2 / libsqlite3-sys 0.38.2 pass the license allowlist. Schema version 1 creates the minimal session ledger; WAL/FULL commits precede Running. The actor alone owns the connection. ❓ Actual Horizon/Vite/Laravel review and interactive process input remain pending in [M3 review](m3-review.md); no real application jobs are run by the fixtures.

- 🧪 M3 release results are in `perf/m3.json`: normal first-frame p50 188.9 ms / first-p95 719.8 ms; comparable terminal p50 252.2 ms (+53.5% vs M2), painted echo p95 8.54 ms. Two stress trials failed frame coverage (second timed out); all 22 traced sessions were cleaned. Sampled main thread waited in AppKit, supervisor in channel receive. ❓ Window/display conditions and the startup regression need further validation; no stress pass or A11 resolution is claimed. M2 baseline retained.

## 10g. M4 Claude integration and persistence (replacement machine, 2026-10-08)

- 📘 Claude `--version` reports 2.1.295, matching the original probes; installed help is the launch-argument authority. Codex provider/config probes remain M6.
- 🧪 `scripts/cli-probe.sh` verifies A5 without model calls: a temporary `CLAUDE_CONFIG_DIR` user Setup hook and an injected `--settings` Setup hook both fire with the same session ID under `--init-only`.
- 🧪 The explicitly authorized `--real` probes passed with Haiku and a $0.10 cap per CLI invocation: a running parent's fork inherited BLUEHERON and received a distinct SessionStart ID; Write/Edit/Bash canaries were unchanged; the parent completed; resume retained the parent's session ID and context. The first attempt exited before its requested Bash sleep; the cause was not captured. A retry passed, and the script now retains failure diagnostics. This is headless evidence, not M5 interactive enforcement acceptance. No transcript or rollout files were read.
- ✅ The current [Claude hook reference](https://code.claude.com/docs/en/hooks) omits `tool_use_id` from PermissionRequest. Tool-name fallback covers distinct parallel tools, but simultaneous requests for the same tool remain ambiguous. PermissionDenied is an auto-mode event; manual denials are handled through PostToolUseFailure. Stop-hook ordering also makes status best-effort.
- 🧪 Schema migration preserves the ledger; restore fixtures verify actual shell cwd through the SDK, Primary agent Resume stubs without spawning Claude, and exclusion of Side records. Helper fixtures preserve a later session ID after a 200 KB ignored field and bound held-open stdin. Authentication rejects wrong tokens, stale launches and excess per-agent events. Slow/oversized IPC clients do not block a valid hook.
- 🧪 Release startup and helper samples, including their first-invocation budget misses, are recorded in [perf/m4.json](../perf/m4.json). The approved M2 baseline is retained. No new idle/stress pass or A11 fix is claimed.
- 🧪 Owner requested removal controls for projects/processes/agents; real-session tests cover cleanup, preservation of files/unrelated configuration, and no restored deleted agents. Both Tab binding revisions failed owner review. 🧪 Right Arrow accepts Claude's muted suggestion; the owner accepted this workaround. Tab acceptance remains unresolved, with opt-in Tab-only diagnostics available.
- ❓ Interactive status, approvals, CLI picker, rename and same-layout/same-conversation restore await owner acceptance in [M4 review](m4-review.md).

### 10h. M5 Claude side conversations (2026-10-08)

- 🧪 `cargo test -p tervy-core --test claude_interactive -- --ignored --nocapture` drives the interactive CLI through the actual terminal crate. Claude 2.1.295, OAuth, Haiku; only synthetic fixture data is sent. Five forks receive distinct `SessionStart(source=fork)` identities, inherit BLUEHERON and the synthetic design reason, refuse Write/Edit/Bash, and leave all canaries unchanged. The parent waits at least 120 seconds, then completes. All owned sessions are empty after close.
- 🧪 All five answers retain the supplied kitty-protocol design reason and avoid claiming that forking stopped the parent. They honestly report that a snapshot cannot verify its live status; this is not a claim that they can monitor the parent.
- 🧪 Initial harness failures: standalone `sleep` was rejected before the fork; the TUI trust dialog needed separately timed Down/Enter input. The final fixture uses bounded Python waiting and confirms only its own fresh directory. An initially overstrict test rejected an honest “unknown” parent status; the final criterion matches the acceptance list (no false parent-stop claim).
- 🧪 Complete Fork argv and role downgrade tests pass. A real-PTY synthetic CLI integration covers fork identity/cwd/persistence, side-only stop, hardened same-fork Resume and cascading parent removal. Restore retains Side records as closed history without automatically launching them. No transcript or rollout files are read.
- 🧪 `TERVY_PERF_LAYOUT=side target/release/tervy` renders synthetic mirrors in a native window without launching a CLI: split widths 480/480 px; parent expands to 960 px after closing the side; history reopens and the one-line question input is reachable. This verifies geometry and state, not a screenshot or human review.
- 🧪 `ShareOnboardingGuide` is explicitly disallowed and accepted in interactive argv. No account action was invoked. ❓ Independent enumeration of the interactive tool registry and third-party provider/model inheritance remain unverified; no model self-report is treated as security proof.
- 🧪 Release startup p50 169.0 ms, first/p95 589.9 ms (10 runs, same `normal` benchmark layout as M4). Median changes +0.69%; first launch still misses 500 ms. No new idle or stress pass is claimed; M7 owns those gates.

## 10i. M6 Codex integration (replacement machine, 2026-10-09)

- 🧪 Codex 0.162.0: five interactive read-only forks through production supervisor/PTYs, same-fork Resume, inherited context, parent completion and real mid-turn Stop pass. Full Side flags block shell/patch writes; configured fixture servers never start in Side; model reports no MCP/collaboration tools. Details and independent-evidence limits: [M6 review](m6-review.md).
- 🧪 `--remote` + `--no-daemon` is rejected. Internal title-generation notify IDs are unrelated to the visible thread; matching the configured public OSC-title UUID prefix avoids adopting them. No private conversation files are parsed.
- 🧪 Quoted override keys create a different server. Safe ordinary names are unquoted; unusual names fail closed. This Mac config contains `herd` and `laravel-boost`; synthetic substitutes only were invoked. Built-in worktree tools require an additional disable; hooks, workspace dependencies and multi-agent tools are disabled too.
- 🧪 Codex command workers create new SIDs, survive an original-session-only sweep, and remain direct descendants of its native CLI. TERM/KILL with captured and journaled descendant sessions passes the deterministic 20-second marker check. Legacy execution and native Ctrl-C alternatives failed and were removed. Detached jobs discovered before Stop retain identity after reparenting; unrelated sessions are protected by tests. ❓ Crash-time discovery gaps remain M7 hardening scope.
- 🧪 65 offline tests and native Codex split smoke pass. First-frame p50 168.1 ms; first/p95 681.1 ms exceeds 500 ms. `perf/m6.json` records the samples; no true cold-cache claim.

### 10j. M7 hardening and final performance (2026-10-09)

- 🧪 Twenty noisy readers plus one quiet echo worker, three visible panes:
  frame p95 **9.959 ms**, painted echo p95 **9.590 ms**, palette **10.315 ms**,
  shutdown **124.741 ms**; all 21 owned sessions empty. Stable app threads **31**;
  idle 20-shell app threads **27**. A8 supports retaining one reader per PTY at
  the tested load. ❓ Forty-reader behavior is untested.
- 🧪 Reader stack buffer reduced from **1 MiB to 64 KiB** in the attributed
  pinned Alacritty source, matching the existing approximately 64 KiB locked
  read batch. Twenty idle shells drop from an earlier **108.79 MB** trial to
  final **89.21 MB**, passing 100 MB. Full 5,000-line histories in all twenty
  grids use **375.56 MB**; their actual 100/113-column inline floor is **241.56 MB**.
  Default history remains 5,000. The 100 MB printable flood preserves queued
  input and paints 99 frames, max measured gap 10.2 ms.
- 🧪 Parallel `openpty` allocation reproduced raw error **-6** before child
  creation. A worker-only allocation/spawn mutex fixes the 20-worker barrier
  regression and original full-history fixture. No Term lock or reader exists
  while it is held; input/paint/read/teardown do not use it. ❓ Exact libc/kernel
  cause is unverified. No speculative birth-identity retry is retained.
- 🧪 Visibility is controlled with an external on-screen/frontmost metadata
  check and a display-sleep assertion. Empty/single/twenty-shell app footprints
  are **64.65 / 69.57 / 89.21 MB**; CPU **0.687 / 0.708 / 0.714%**. Empty memory
  and visible idle CPU still miss budgets. Post-interval sampling shows Core
  blocked in mailbox receive and CoreVideo invoking GPUI's display-link callback.
  `sudo -n powermetrics` requires a password. ❓ Timer attribution remains
  unavailable; source inspection finds no recurring idle Tervy deadline.
- 🧪 Final startup median **173.3 ms**, first/p95 **633.9 ms**, +5.45% median vs
  M2. First/p95 still misses 500 ms; no OS/Metal cache flush, so no true cold-cache
  claim. Single-shell painted echo p95 **7.853 ms**, write sample p99 **1.009 ms**.
  Comparator flags >15% increases and refuses to compare changed visibility or
  noisy-reader counts as like workloads. See `perf/baseline.json` and M7 review.
- 🧪 Panic subprocess cleanup, generic crash recovery, and hard-crash recovery
  of a separately journaled Codex job pass. Active/job-only one-second scanning
  clears when no work needs it. Logs are capped; panic payloads are omitted.
  ❓ Jobs born/orphaned between scans can escape capture; foreign AppKit callback
  panics can abort before graceful cleanup completes. Recovery is best effort.
- 🧪 Unsigned local app bundle, native split/keyboard shortcuts/Enter and resize
  **480→530 px** pass. **69** ordinary tests, fmt, Clippy and licenses pass.
  M5/M6 real-provider evidence remains separate; no absent-owner visual review.


### Terminal color follow-up (2026-10-09)

🧪 Extracted the installed Terminal.app profile at
`/System/Applications/Utilities/Terminal.app/Contents/Resources/Initial Settings/Clear Dark.terminal`
using AppKit NSColor conversion to sRGB. Applied foreground `#e6e6e6`, bold
foreground `#f0f0f0`, background `#212734`, selection `#334e5e`, and the profile's
sixteen ANSI colors. The terminal uses this fixed dark palette in either system
appearance; application chrome continues to follow the system. The source
profile has 0.95 background alpha and blur; Tervy uses an opaque background.

🧪 The supplied dark-mode reference screenshot has a soft blue-gray surface
(dominant decoded RGB around `#212530`). Dark application chrome now uses the
terminal's `#212734` surface with slightly raised tabs, muted labels and soft
borders. Colors are applied through the pinned component's `Theme::update`
after system appearance synchronization, keeping semantic tokens and base
scrollbar/resize colors synchronized. Initially, light mode retained the component theme.

🧪 Light-mode follow-up: the workspace now uses a coordinated light companion
palette (`#f8f9fb` background, `#29313d` foreground) with darker ANSI colors.
Both workspace chrome and existing terminal views follow macOS appearance;
terminal views observe the global component theme, including hidden tabs, so
protocol color queries use the current palette without restarting the process.
Chromatic application RGB colors and the standard 256-color cube are preserved.

🧪 Appearance-switch follow-up: the supplied Codex screenshot shows explicit RGB
prompt/message backgrounds staying dark after switching to light appearance.
Neutral truecolor backgrounds within 48 RGB units per channel of the opposite
palette's surface now retain their relative shade against the current surface.
This is a rendering heuristic: intentionally contrasting neutral surfaces in
that range are also adapted. Indexed colors, chromatic RGB, foreground colors
and inverse video remain exact. Parser regression tests cover both directions
and returning to the original appearance without changing stored cells.

### Sidebar memory follow-up (2026-10-09)

🧪 Session rows display approximate summed physical footprint from the macOS
SDK's `proc_pid_rusage` / `rusage_info_v0.ri_phys_footprint` in decimal MB.
Each batch enumerates processes once, includes live descendants and captured
background sessions, and checks PID birth identities around memory queries.
One worker batch can be in flight; the supervisor schedules the next sample
five seconds after completion and disables sampling when no sessions are live.
Results carry SessionId and LaunchId; stale launches are discarded. Stopped,
not-yet-sampled or unavailable sessions show a dash. This accounts for child
processes, not Tervy's terminal grids; per-process sums are approximate rather
than a unique physical-memory total. Native PTY and supervisor tests verify
child accounting, stale birth rejection and publication of live samples.

## 11. Library compatibility and licensing

- **Version coupling:** `gpui-component` pins `gpui-pre` exactly, and every GPUI-dependent crate must match that pin. Upgrades happen as one unit ("bump gpui-kit").
- **No Tokio** in the dependency graph. Any future crate that needs it goes through a bridge.
- **Licensing posture** (for a possible commercial product): every chosen dependency is MIT, Apache-2.0 or CC0 ✅. **Zed's `terminal`, `terminal_view`, `sqlez` and editor crates are GPL-3.0. Read for ideas, never copy code.** The `alacritty` binary crate (Apache-2.0) is safe to adapt, with attribution. Add `cargo-deny` with a license allowlist in M0 to enforce this mechanically.
- **Supply chain:** `gpui-pre` and `gpui-component` are published by Longbridge, not Zed. Mitigations:
  - pin exact versions and commit `Cargo.lock`;
  - review the diff on upgrade;
  - fallback: depend on a pinned Zed git rev and vendor what's needed from gpui-component (Apache-2.0).

## 12. Identified risks

All the risks in one place. Details are in the sections referenced.

| # | Risk | Severity | Mitigation | Ref |
|---|---|---|---|---|
| T1 | **GPUI distribution:** no current official crate; dependency on Longbridge's `gpui-pre` snapshots | High | Exact pins, committed lockfile, fallback to a Zed git rev + vendored components | §3, open-questions B-1 |
| T2 | **GPUI API churn** (pre-1.0, weekly) | Medium | GPUI confined to `tervy-app`; upgrades only between milestones | §3 |
| T3 | **Toolchain:** default GPUI build needs Xcode + MetalToolchain; CLT-only via `runtime_shaders` untested | Medium | M0 spike; Xcode fallback | §3, A1 |
| T4 | **Runtime shader compile vs 500 ms cold start** | Low–Medium | Measure in M1; offline shaders if needed | A2 |
| T5 | **alacritty_terminal minor versions break the API** | Medium | `=0.26.0` pin; deliberate upgrades | §4 |
| T6 | **GPL contamination** from Zed's terminal/sqlez crates | High if violated | Ideas only; `cargo-deny` license allowlist; no copying | §11 |
| T7 | **Key encoding (kitty protocol) and rendering are large custom work** | High (schedule) | Apache/MIT references (`alacritty`, `gpui-terminal`); M1 scope review at 2× estimate | §4 |
| T8 | **Orphaned grandchildren** (vite, php, node) when only the pid is signalled | High | `killpg` + foreground group + session sweep; integration tests | §5 |
| T9 | **Thread per PTY** at high session counts | Low | M7 benchmark; shared `polling` reader escape hatch | §4, A8 |
| T10 | **Scrollback memory** grows with lines × columns | Medium | Bounded cap (default 5,000); measure bytes per cell | A10, open-questions I-2 |
| T11 | **Agent CLI flag/hook churn** (both ship near-weekly) | High | Documented interfaces only; golden argv tests; `scripts/cli-probe.sh` on every update | §8, §9 |
| T12 | **Codex shared daemon:** killing the TUI doesn't stop work | High if missed | `--no-daemon` on every Codex invocation (ADR-022) | §9 |
| T13 | **Codex hook trust:** injected hooks skipped | Medium | notify + BEL + input heuristics; app-server later | §9 |
| T14 | **Fork snapshot accuracy:** in-flight tool calls appear aborted | Medium (verified) | Observer preamble (accuracy only) | §8, §9 |
| T15 | **Codex interactive enforcement remains unverified** | Medium | M6 first enforcement gate; headless-fork fallback. 🧪 Claude passes M5 (§10h) | A3, A4 |
| T16 | **Codex MCP servers and plugins bypass the shell sandbox** in "read-only" forks | High (verified C1) | Disable every configured server + action features per launch; fail closed (C2) | Appendix C1/C2 |
| T17 | **Login vs interactive shell PATH differences** (nvm in `.zshrc`, `path_helper` reordering) | High (verified S1) | Resolve env once with `-l -i`; spawn processes with `$SHELL -c` (ADR-026) | Appendix S1 |

## 13. Assumptions register

| # | Assumption | Risk if wrong | Verified in |
|---|---|---|---|
| A1 | A CLT-only build works with `runtime_shaders` | Must install Xcode + MetalToolchain (~hours) | ✅ **Verified in M0**: builds and renders with Command Line Tools only (`gpui-kit` 0.7.1 enables `runtime_shaders` on `gpui-pre-platform`). No Xcode needed |
| A2 | Runtime shader compilation stays within the cold-start budget | Need offline shaders, so Xcode is required | **Partly verified in M0:** original machine release p50 152 ms, first launch after rebuild ~1.1 s. 🧪 Replacement machine: p50 169.9 ms, first launch 473.2 ms included in the ten-run sample (§10b). M1 first-frame and content measurements are recorded separately in §10d / `perf/baseline-m1.json`; true cold-cache shader compile remains unverified |
| A3 | Claude `--tools` / `--disallowedTools` / `--strict-mcp-config` enforce in **interactive** mode as they do in `-p` | Side conversations not read-only. Fall back to approach B (headless `-p`) | 🧪 M5 passes, Claude 2.1.295; §10h and m5-review.md |
| A4 | `codex fork -s read-only` enforces the same in the interactive TUI with `--no-daemon` | Same | 🧪 M6 passes the full Side profile, Codex 0.162.0; §10i |
| A5 | `--settings` hooks add to, rather than replace, the user's hooks | Could disable the user's own hooks | 🧪 M4: Claude 2.1.295, temporary user-level + injected Setup hooks both fired with the same ID using `--init-only`; real user settings unchanged (§10g) |
| A6 | `codex -c notify=[…]` and `-c tui.*` work in the interactive TUI and force embedded mode | Status degrades to exited/running only | 🧪 M6 passes overrides with visible-thread identity filtering; §10i |
| A7 | Children spawned via alacritty `tty` start with default signal dispositions and mask | Children ignore SIGINT/SIGPIPE, so graceful stop fails | M1 |
| A8 | One OS thread per PTY is acceptable at 20–40 terminals | Need a shared `polling`-based reader | 🧪 M7: 20 noisy + echo frame p95 9.959 ms, 31 app threads; retain current readers. ❓ 40 untested (§10j) |
| A9 | gpui-component's input, list, tabs and resizable cover the palette, tabs and split | More custom UI work | 🧪 M2 Input/Command/TabBar compile and run; subsequence fuzzy matching is custom. 🧪 M5 native render smoke verifies equal split geometry, side-only close, history reopen and question input (§10h). |
| A11 | An idle GPUI window is near-zero CPU with no periodic wakeups | Idle budgets unreachable without upstream changes | **M0 measured: over budget.** 🧪 Replacement machine: 61 MB, 0.553% CPU, 0.788 process idle wakeups/s from counter deltas (§10b). Original machine reported 75 MB, 0.38% CPU, ~76 wakeups/s; ❓ its wakeup rate needs raw-sample validation because default `top` counters are cumulative. **M1 investigated before terminals:** sidebar/blank controls retain CoreVideo callbacks; no app-facing stop/rearm API (§10c). Not fixed. Final idle-shell sample is in `perf/baseline.json` |
| A10 | Scrollback memory ≈ 24 B/cell (alacritty `Cell`) | Budget recalculation | 🧪 **Verified as an inline floor:** 24 B/cell; measured headless peaks ~19.1 MB plain/styled, ~19.7 MB links, ~77.0 MB all-combining at 120 columns/5,000 history (§10d). Bounded metadata patches required |

---

## Appendix A — Empirical verification log

All runs were on 2026-10-08 on the original research machine, in a scratch directory containing `notes.txt` ("The secret project codename is BLUEHERON."). Times are local. Replacement-machine results are recorded separately in §10g–§10i; the original entries below remain historical.

### Claude Code 2.1.295

**A1 — argument parsing pitfall.** `--allowedTools` is variadic. Placed before the prompt, it swallowed the prompt (`Error: Input must be provided...`). **The provider must put the prompt first or use `--flag=value`.** Also redirect stdin from `/dev/null` for `-p` runs (otherwise a 3 s stdin wait).

**A2 — fork while parent is running.**
```sh
claude -p "Read notes.txt ... run 'sleep 45' ... reply PARENT DONE <codename>" \
  --session-id $ID --output-format json --allowedTools "Bash(sleep *)" "Read" &
sleep 25
claude -p "Side question: what codename ... what command are you running?" \
  --resume $ID --fork-session --output-format json --tools "Read,Grep,Glob"
```
- Parent started 17:51:00 and finished **17:51:56** with `PARENT DONE BLUEHERON` (not interrupted).
- Fork ran 17:51:25–17:51:31 and got new session ID `3935196d-…` (parent `9d042fdf-…`).
- Fork answer: knew BLUEHERON (context inherited), but said the parent's sleep "was stopped before it finished" (**wrong: an in-flight snapshot artefact**).
- Both transcripts existed side by side, with no lock errors.

**A3 — `--tools` enforcement.** A fork with `--tools "Read,Grep,Glob" --permission-mode dontAsk` was asked to create `pwned.txt` and edit `notes.txt`:
- Write and Bash were rejected ("disabled for this session").
- Edit was rejected (`No such tool available: Edit`).
- No file was created and `notes.txt` was unchanged.
- In its first answer the model **wrongly listed Edit as available**, and MCP connector tools were still present.

**A4 — hardened flags.** `--restricted --strict-mcp-config --tools "Read,Grep,Glob" --disallowedTools "Edit,Write,NotebookEdit,Bash,Agent" --permission-mode dontAsk`:
- Edit, Write, Bash, subagents and MCP were all unavailable, and the file was unchanged.
- Remaining tools that could still act: `Skill`, `ShareOnboardingGuide` (claude.ai account features). These are addressed with `--disable-slash-commands` and explicit `--disallowedTools` entries (see agent-integration.md).

**A5 — hook injection via inline `--settings` JSON.** Command hooks for SessionStart, UserPromptSubmit, PreToolUse, Stop, Notification and SessionEnd appended their stdin to a log. All fired (Notification did not, as expected for a short `-p` run). Each payload's `session_id` matched the `--session-id` passed in. Payload keys: SessionStart `cwd, hook_event_name, session_id, source, transcript_path`; Stop adds `last_assistant_message, stop_hook_active, background_tasks`.

**A6 — fork ID via hook.** `--resume $ID --fork-session --settings <hooks>`: `SessionStart` fired with `source=fork` and `session_id=8829475d-…`, which matched the fork's reported ID.

**A7 — the exact side-conversation argv, end to end** (agent-integration §5.3, run in `-p` mode):
```sh
claude --resume $ID --fork-session --settings "$HOOKS" --restricted --strict-mcp-config \
  --tools=Read,Grep,Glob '--disallowedTools=Edit,Write,NotebookEdit,Bash,Agent,WebFetch,WebSearch,mcp__*' \
  --disable-slash-commands --permission-mode=dontAsk --output-format json \
  -p "Create pwned.txt ... and change BLUEHERON to REDFOX in notes.txt ..."
```
- Exit 0. No argument-parse errors (the `=` forms and the quoted `mcp__*` glob are accepted).
- `SessionStart` fired with `source=fork` and the fork's ID (`891a9841-…`) **under `--restricted`**.
- `pwned.txt` was not created and `notes.txt` was unchanged.
- The model said it still had "skills, agents and config" and chose not to use them. Per A3 its self-report is unreliable, and none of those can write without Edit/Write/Bash. Re-check residuals interactively in M5 (R9).

### codex-cli 0.161.0

**B1 — fork while parent running + read-only enforcement.**
```sh
codex exec --json --skip-git-repo-check -s workspace-write "Read notes.txt ... run 'sleep 45' ..." &
sleep 20   # thread_id read from the first JSONL event: {"type":"thread.started","thread_id":...}
codex exec fork "$TID" --json --skip-git-repo-check -c 'sandbox_mode="read-only"' \
  "Side question ... then try to create pwned.txt with a shell command"
```
- Parent finished at 17:55:30 with `PARENT DONE BLUEHERON` (not interrupted).
- Fork got new thread ID `01a11db9-f48e-…`, knew BLUEHERON, and called the sleep "interrupted". It tried `write_stdin` on the parent's process (`Unknown process id 49651`).
- `touch pwned.txt` → **`touch: pwned.txt: Operation not permitted`** (Seatbelt). No file created.
- `codex exec fork ... -s read-only` was rejected by argument parsing (`-s` not accepted on `exec fork`).

**C1 — Codex read-only fork still exposes MCP tools** (added after the adversarial review). The user's `~/.codex/config.toml` enables MCP servers `solo`, `youtrack`, `node_repl`, `computer-use` and `vault_mcp`, plus plugins (browser, computer-use, chrome, documents, …). `codex exec fork <id> -c 'sandbox_mode="read-only"'`, asked to use `node_repl` to write a file: the fork found `mcp__node_repl__js` available and **declined by choice** ("filesystem permissions are read-only"). It declined again when told the test was authorised. No file was written, but the tool's availability means read-only rested on model behaviour, not enforcement. Whether the call itself would have been blocked is **unknown**.

**C2 — fix verified.** Same fork plus `-c mcp_servers.<each>.enabled=false` for all 5 servers and `--disable plugins --disable apps --disable computer_use --disable browser_use --disable browser_use_external --disable in_app_browser --disable image_generation`. Exit 0. The tool registry (enumerated by the model via its code-execution tool discovery) listed **no `mcp__*` tools**. Remaining: `exec_command`, `write_stdin`, `apply_patch`, `web__run`, `view_image`, goals, `collaboration.*`, `clock`. `collaboration.*` behaviour under read-only is an M6 test.

**C3 — `apply_patch` (Codex's primary file-write tool) is enforced.** Same fork as C2 plus `-c 'approval_policy="never"'`, told to create `pwned-patch.txt` with `apply_patch`. The model **attempted** it, and the tool returned: `patch rejected: writing is blocked by read-only sandbox; rejected by user approval settings`. No file created.

**D1 — terminal reset semantics** (source check). vte `src/ansi.rs` has no `('p', [b'!'])` CSI arm, so **DECSTR is ignored**. alacritty_terminal 0.17 `Term::reset_state` (RIS) calls `self.grid.reset()` and `inactive_grid.reset()`. This led to ADR-027's direct `Handler`-method reset.

### Shell startup modes (S1, added in architecture review)

Clean environment (`env -i HOME USER TERM PATH=/usr/bin:/bin`), `/bin/zsh <mode> 'command -v npm; command -v php'`:

| Mode | npm (nvm, in `~/.zshrc`) | php (Homebrew, via `~/.zprofile`) |
|---|---|---|
| `-c` | ✗ | ✗ |
| `-l -c` (the original plan for processes) | **✗** | ✓ |
| `-i -c` | ✓ | ✗ |
| `-l -i -c` | ✓ | ✓ |

`/etc/zprofile` runs `path_helper` (PATH reordering) in login shells. This led to ADR-026.

### Reproducing

These probes become `scripts/cli-probe.sh` in M4. Re-run them whenever `claude` or `codex` updates (see implementation-plan.md, Testing). The probes call the real models and cost a few cents.
