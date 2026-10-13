# Tervy — Architecture Decision Records

Status values:
- **Accepted:** decided. Product decisions were made with the project owner in the 2026-10-08 planning interview.
- **Proposed:** recommended; needs owner sign-off.
- **Pending:** deliberately open; see [open-questions.md](open-questions.md).
- **Rejected:** considered and declined, recorded so it isn't reopened without new information.

Each ADR uses the format: Context → Decision → Consequences → Revisit when.

## Index

| ADR | Title | Status |
|---|---|---|
| [001](#adr-001-native-rust) | Native Rust application | Accepted |
| [002](#adr-002-gpui-via-gpui-component) | GPUI, consumed via the gpui-kit facade (gpui-pre pin) | Accepted (2026-10-08) |
| [003](#adr-003-terminal-emulation-alacritty_terminal) | Terminal emulation: alacritty_terminal + own renderer | Accepted |
| [004](#adr-004-pty-management) | PTY management: alacritty `tty`; Tervy owns group kills | Accepted |
| [005](#adr-005-in-process-ownership-no-daemon) | In-process ownership, stop all on quit, no daemon | Accepted |
| [006](#adr-006-no-async-runtime-in-core) | No async runtime in core | Accepted |
| [007](#adr-007-agents-as-embedded-tuis) | Agents as embedded TUIs; providers are pure | Accepted |
| [008](#adr-008-side-conversations-via-native-fork) | Side conversations via native fork, read-only enforced | Accepted |
| [009](#adr-009-agent-status-signals) | Agent status via injected hooks / notify + BEL | Accepted |
| [010](#adr-010-persistence-toml--sqlite) | Persistence: hand-edited TOML + SQLite | Accepted |
| [011](#adr-011-three-crate-workspace) | Three-crate workspace | **Proposed** |
| [012](#adr-012-session-discovery-scope) | Track only Tervy-launched sessions | Accepted |
| [013](#adr-013-restore-semantics) | Restore UI state + resumable stubs | Accepted |
| [014](#adr-014-provider-rollout-order) | Claude first, Codex immediately after | Accepted |
| [015](#adr-015-no-secret-store-in-mvp) | No secret store in MVP | Accepted |
| [016](#adr-016-bounded-in-memory-output) | Bounded in-memory scrollback only | Accepted |
| [017](#adr-017-shared-project-directory) | Agents share the project dir; worktrees deferred | Accepted |
| [018](#adr-018-window-close-vs-quit) | Close hides; Cmd-Q confirms and stops | Accepted |
| [019](#adr-019-name) | Product name Tervy | Accepted |
| [020](#adr-020-permissive-licensing-only) | Permissively-licensed dependencies only | Accepted |
| [021](#adr-021-processes-run-in-ptys) | Dev processes run in PTYs, not pipes | Accepted |
| [022](#adr-022-codex-always---no-daemon) | Codex always launched with `--no-daemon` | Accepted |
| [023](#adr-023-reject-codex-hook-trust-bypass) | Do not use `--dangerously-bypass-hook-trust` | Rejected (as default) |
| [024](#adr-024-no-per-repo-config-in-mvp) | No per-repo `.tervy.toml` in MVP | Accepted |
| [025](#adr-025-side-conversation-tool-scope) | Claude side conversations: read/search only, no shell | Accepted |
| [026](#adr-026-process-spawn-mode) | Processes spawn as `$SHELL -c` with the pre-resolved env | Accepted (review fix) |
| [027](#adr-027-process-identity-outlives-instances) | A process's scrollback outlives its instances | Accepted (review fix) |

---

### ADR-001: Native Rust
**Context:** The brief requires a fast, low-memory, non-Electron desktop app.
**Decision:** Rust for the whole application.
**Consequences:** Excellent performance and memory control. Strong compile-time guarantees around shared state (important with PTY threads). Slower early iteration than a scripting stack. The owner is new-ish to Rust, so the code stays idiomatic and avoids clever generics.
**Revisit when:** Never for the core.

### ADR-002: GPUI via gpui-component
**Context:** GPUI is the only Rust UI framework with a proven high-throughput terminal (Zed). The official `gpui` crate is stale (0.2.2, Oct 2025). Current GPUI is available only as a Zed git rev or a third-party snapshot (`gpui-pre`, published weekly by Longbridge). `gpui-component` provides input, list, command palette, tabs and resizable splits, and pins `gpui-pre` exactly.
**Decision (accepted 2026-10-08):** Depend on the **`gpui-kit` facade** (0.7.1, Apache-2.0 ✅). It brings matching `gpui-component`, `gpui-pre` and platform crates, whereas a transitive `gpui-pre` isn't directly importable (adversarial review). Pin it exactly, enable `runtime_shaders`, keep all GPUI usage inside `tervy-app`, and record the compiling manifest and platform initialisation in M0.
**Alternatives:**
- (a) Pin a Zed git rev and build widgets by hand: no third-party publisher, but weeks of widget work.
- (b) egui or iced: no comparable terminal reference; harder to hit the idle-CPU target.
**Consequences:** Supply-chain dependency on Longbridge's snapshot cadence. Upgrades are all-or-nothing ("bump gpui-kit"). Fallback: switch to a Zed git rev and vendor the needed gpui-component pieces (Apache-2.0).
**Revisit when:** Zed resumes official crates.io releases, or gpui-component stalls for more than 2 months.

### ADR-003: Terminal emulation: alacritty_terminal
**Context:** We need a correct VT emulator with bounded scrollback, and a permissive license.
**Decision:** `alacritty_terminal =0.26.0` for parsing and grid state. Tervy writes its own GPUI renderer (batched text runs and background rects) and key encoder (kitty protocol), adapted from the Apache-2.0 `alacritty` crate and MIT/Apache `gpui-terminal`. Zed's GPL terminal code is never copied.
**Rejected:** `wezterm-term` (git-only dependency), `vt100` (too limited), `libghostty-vt` (Zig build dependency, pre-1.0; revisit 2027).
**Consequences:** Must answer terminal queries (PtyWrite, ColorRequest) ourselves. Pin exactly, because minor versions break.
**Implementation evidence (M1, 🧪):** A10 confirmed unbounded per-cell/OSC metadata in the pinned registry sources. Narrow attributed local patches bound that metadata without changing the versions or APIs; see [vendor/README.md](../vendor/README.md) and technical-research §10d.
**Revisit when:** libghostty offers a stable, pure-cargo build.

### ADR-004: PTY management
**Context:** The brief suggested `portable-pty`, but `alacritty_terminal` includes a PTY spawner (`setsid` + controlling TTY) and a reader event loop already wired to `Term`.
**Decision:** Use alacritty's `tty` + `EventLoop`. Drop `portable-pty`. Tervy implements process-group termination (`killpg`, `tcgetpgrp`, session sweep), which neither crate provides.
**Consequences:** One fewer dependency layer. One OS thread per PTY (fine at 20–40; a shared `polling` reader is the escape hatch).
**Revisit when:** The M7 benchmark shows thread count or wakeups matter.

### ADR-005: In-process ownership, no daemon
**Context:** Should processes outlive the app? A daemon means IPC, PTY handoff, reattach, and an IPC security surface. The owner chose stop-on-quit.
**Decision:** The Tervy process owns all children. Cmd-Q confirms, then gracefully stops everything (SIGINT/SIGTERM → SIGKILL, 15 s global deadline). Agents remain resumable because the CLIs persist their conversations.
**Seam for later (corrected in review):** All lifecycle logic lives in `tervy-core::supervisor` behind a `Command`/`CoreEvent` message interface with no GPUI types. That covers **lifecycle** only. **Rendering is not behind the seam:** the UI reads `Arc<FairMutex<Term>>` in-process. A daemon would therefore also need either:
- (a) the UI to own `Term`, fed raw PTY bytes forwarded by the daemon, with a replay buffer for reattach (the cheaper path, and the same change as the "own reader loop" escape hatch in ADR-004); or
- (b) grid sync.

Estimate: a daemon is an L-sized project, not a transport swap. No change now (it's plausible, not likely), but `core` must keep never touching `Term` internals.
**Triggers to revisit:** The owner regularly wants dev servers running with the app closed, **or** a crash-recovery requirement appears that the session ledger can't meet.

### ADR-006: No async runtime in core
**Context:** The brief proposed Tokio. GPUI has its own executors. Zed confines Tokio to a small bridge crate. Tervy's I/O is PTYs (alacritty threads), one Unix socket, SQLite (sync) and a file watcher.
**Decision:** The core uses threads + channels (`async-channel` toward the UI). No Tokio, no smol in core. The UI awaits core events on GPUI's foreground executor.
**Consequences:** One scheduling model, trivially testable. If a Tokio-only crate becomes necessary, add a `gpui_tokio`-style bridge in the app crate only.

### ADR-007: Agents as embedded TUIs
**Context:** A native chat UI (via stream-json, SDKs or app-server) is months of UI work and couples Tervy to protocol churn.
**Decision:** Run `claude` / `codex` interactively in Tervy terminal panes. Providers are **pure** (they build argv and interpret signals); the supervisor owns processes. One code path builds the complete argv for New, Resume and Fork, because resume doesn't persist flags.
**Consequences:** Full fidelity, free CLI updates. Status must come from out-of-band signals (ADR-009).

### ADR-008: Side conversations via native fork
**Context:** The key differentiator. Verified empirically on 2026-10-08: both CLIs can fork a **running** session, the parent continues uninterrupted, and read-only is enforced (Claude tool removal: Edit/Write/Bash rejected; Codex Seatbelt: `Operation not permitted`).
**Decision:** "Ask Question" opens a split pane running a native fork with enforced read-only flags. The question goes in the initial prompt with an observer preamble.
- **Read-only covers every tool surface, not just the shell:**
  - **Claude:** `--tools` + `--disallowedTools` + `--strict-mcp-config` + `--restricted` (🧪 A7).
  - **Codex:** `-s read-only` covers only shell commands. The adversarial review found MCP servers (`node_repl`!) and plugins still exposed (🧪 C1). Every configured MCP server and the action-capable features are disabled per launch (🧪 C2), failing closed if the user's config can't be read.
- The permission profile is derived from the record's role on **every** launch path (`LaunchProfile`).
**Fallback:** If interactive-mode enforcement fails verification, run the fork headlessly (`-p --output-format stream-json`, verified). That is effectively a **mini native chat**: input box, streamed text, and `--resume <fork-id>` for each follow-up. Size **M**, not trivial.
**Consequences:** The fork sees in-flight tool calls as aborted (mitigated by the preamble; accuracy only). Each question re-sends the parent's context (cost). The snapshot is point-in-time.

### ADR-009: Agent status signals
**Decision:**
- Claude: hooks injected via `--settings` JSON (SessionStart, UserPromptSubmit, PermissionRequest, PostToolUse, Notification, Stop, SessionEnd) → Tervy hook helper → Unix socket. Verified locally.
- Codex: `-c notify` (turn complete + thread ID) + `tui.notifications=["approval-requested"]` via BEL + "Enter pressed" → working.
- Process exit is authoritative for both.
- The user's settings files are never modified.

**Consequences:** Codex status is heuristic. Upgrade path: a Tervy-owned Codex app-server. 🧪 M6 (0.162.0): internal title-generation turns also notify. Correlate notify IDs with the configured public terminal-title UUID prefix; retain explicit inferred/Attention labels. `--remote` and `--no-daemon` are incompatible, so the spike retains the accepted default.

### ADR-010: Persistence: TOML + SQLite
**Decision:** User-authored `config.toml` (projects, processes, settings), hot-reloaded. Tervy appends projects via `toml_edit`. Owner-authorized M4 extension: sidebar Remove controls also delete project/process definitions with confirmation, preserving unrelated comments and project files. `state.db` (rusqlite, bundled, WAL) holds agent records, UI layout and the session ledger, and is written only by the supervisor thread.
**Rejected:** YAML (no maintained serde stack, whitespace-fragile); sqlx (Tokio); a JSON state file (no atomic multi-record updates or queries as history grows).

### ADR-011: Three-crate workspace
**Decision (proposed):** `tervy-term` (no GPUI, no domain), `tervy-core` (no GPUI), `tervy-app` (GPUI binary + hook helper subcommand). Modules inside them replace the brief's 7 crates.
**Consequences:** The boundaries that matter (UI vs lifecycle, terminal vs domain) are compiler-enforced; the rest are module boundaries.
**Revisit when:** A module needs independent reuse (e.g. a daemon binary) or compile times hurt.

### ADR-012: Session discovery scope
**Decision:** List only Tervy-launched sessions. "Resume other session…" opens the CLI's own picker. The picked session's ID arrives via hooks/notify and becomes a record. No transcript or rollout parsing.

### ADR-013: Restore semantics
**Decision:** On relaunch, restore the active project and its tab layout. Agent tabs that were open come back as **Resume stubs**. Shell tabs come back as fresh shells in the same cwd. Autostart processes start on the **first open of their project** in this app run. Side conversations aren't reopened. Tervy never claims to restore running processes.

### ADR-014: Provider rollout order
**Decision:** Claude provider (including side conversations) in M4–M5, Codex in M6. The provider trait is validated against a second real provider before being considered stable.

### ADR-015: No secret store in MVP
**Decision:** Children get the login-shell environment + non-secret overrides from config. Projects use their own `.env`. Agent CLIs manage their own auth. Designed for later: `env.KEY = { keychain = "service/account" }`, resolved at spawn time.

### ADR-016: Bounded in-memory output
**Decision:** Per-terminal scrollback cap (default 5,000 lines; configurable). No log files of process output. Tervy's own diagnostics log (`~/Library/Logs/Tervy`) is separate and size-capped.
**Revisit when:** The owner needs crash output to survive an app restart. Then add rotating per-process logs, written off the hot path.

### ADR-017: Shared project directory
**Decision:** Agents run in the project directory (or a user-specified path, e.g. a worktree they made). Tervy does not create or manage git worktrees in the MVP.
**Revisit when:** Parallel agents on one project become routine (note: both CLIs now have `--worktree` flags, which may make this cheap).

### ADR-018: Window close vs quit
**Decision:** Closing the window hides it (processes keep running, app stays in the Dock). Cmd-Q shows a confirmation listing running sessions, then stops all of them.

### ADR-019: Name
**Decision:** "Tervy" for crates, bundle (`com.jringeisen.Tervy`) and App Support paths. The earlier local state directory is migrated on first launch. A trademark check is deferred to the commercial phase.

### ADR-020: Permissive licensing only
**Context:** Personal tool now, possibly commercial later.
**Decision:** Dependencies must be MIT, Apache-2.0, BSD, ISC, Zlib, CC0 or Unicode, enforced by `cargo-deny` in M0. Zed's GPL crates (`terminal`, `terminal_view`, `sqlez`, editor) are reference reading only, and no code is copied from them.

### ADR-021: Processes run in PTYs
**Decision:** Dev processes run in PTYs (`$SHELL -c "<command>"` with the resolved env; see ADR-026), rendered by the same terminal component as shells.
**Consequences:** Colours, TTY-detection-dependent behaviour and interactive input (`tinker`, prompts) work. A small extra memory cost per process.

### ADR-022: Codex always `--no-daemon`
**Context:** Codex's TUI attaches to a shared daemon by default; killing the TUI leaves the turn running. `-s` / `-a` don't force embedded mode.
**Decision:** Every Codex argv Tervy builds (new, resume, fork) includes `--no-daemon`. Tervy never stops the user's Codex daemon.

**M6 evidence / consequence:** 🧪 In 0.162.0, command workers create additional SIDs and survive an original-SID-only stop. Before Stop, capture and journal live descendant sessions by kernel ancestry and birth identity, then sweep them too. Recover a surviving Codex TUI's descendants before terminating it. No process-name matching or global daemon stop. This strengthens lifecycle enforcement without removing `--no-daemon`.

### ADR-023: Reject Codex hook-trust bypass
**Context:** Hooks injected via `-c` are untrusted and skipped. `--dangerously-bypass-hook-trust` runs **all** non-managed hooks, including untrusted repo hooks.
**Decision:** Not used by default. It could become an explicit, documented opt-in setting if notify + BEL proves insufficient and app-server is still experimental.

### ADR-024: No per-repo config in MVP
**Decision:** Only the user's global config defines commands. Opening a folder never executes anything.
**Future:** `.tervy.toml` import requires a trust prompt showing exact commands, with trust pinned to a content hash (re-prompt on change).

### ADR-025: Side conversation tool scope
**Decision:** Claude side conversations get Read/Grep/Glob only, with no shell. Codex side conversations run under the read-only Seatbelt sandbox, which allows read-only shell commands. This asymmetry is accepted.
**Revisit when:** A verified recipe exists for read-only Claude Bash (sandbox `denyWrite` on the project + Bash allowlist).

### ADR-026: Process spawn mode
**Context:** The original plan used `$SHELL -l -c "<command>"`. Measured on this Mac, that misses nvm's `npm`, because nvm lives in `~/.zshrc`, which only interactive shells read. Login shells also run `path_helper`, which reorders PATH.
**Decision:** Resolve the environment once with `$SHELL -l -i -c 'env -0'` (3 s timeout, background thread). Spawn processes as `$SHELL -c "<command>"` (non-login, non-interactive) with that environment. Interactive shell tabs stay `$SHELL -l`. Agents are exec'd directly with the resolved env.
**Consequences:** Deterministic PATH. A change to rc files needs "Reload shell environment" (palette command) or an app restart.
**Revisit when:** A user's setup depends on per-command interactive shell features (aliases or functions in process commands). Such commands can opt in with `shell = "interactive"`.

### ADR-027: Process identity outlives instances
**Context:** Restart-on-failure created a new `Term`, erasing the output that explains the failure.
**Decision:** A configured process owns one `Term` for the lifetime of the app run. Each start is a new instance (PTY, pid, `LaunchId`, EventLoop) attached to the same `Term`, after joining the old reader, resetting the modes a crashed program left behind, and writing a separator line.
**How modes are reset (verified in final review):** **not** via escape sequences.
- ✅ vte has no DECSTR (`ESC[!p`) handler, so it would be silently ignored.
- ✅ RIS (`Term::reset_state`) calls `grid.reset()`, which would wipe the history this ADR exists to keep.

Tervy instead calls `Term`'s public `Handler` methods directly: leave the alternate screen, reset the keyboard-mode stack, disable mouse/focus/bracketed-paste modes, reset cursor style and scroll region. 🧪 M3 task 3.2b verifies the exact method set with a real-PTY test: a crashed TUI in the alt screen with mouse + kitty modes → restart → history intact and modes default.
**Consequences:** Crash output stays visible above the restarted run. Scrollback is shared across restarts, within the same cap.
