# Tervy — Open Questions

Most product questions were settled in the 2026-10-08 planning interview (recorded as Accepted ADRs in [decisions.md](decisions.md)). Questions that research could answer were answered in [technical-research.md](technical-research.md). Its ❓ items are verification tasks with assigned milestones, not open questions. What remains is below.

| ID | Question | Category | Recommendation |
|---|---|---|---|
| B-1 | Which GPUI source? | ✅ Resolved 2026-10-08 | gpui-kit facade (pinned gpui-pre) |
| B-2 | May I install the Rust toolchain (and Xcode if needed)? | ✅ Resolved 2026-10-08 | rustup now; Xcode only if the CLT build fails |
| I-1 | Default stop signal for processes | Important | SIGINT |
| I-2 | Default scrollback size | Important | 5,000 lines |
| I-3 | macOS notifications for "needs approval" | Important | Dock badge always; notification only when the pane isn't visible |
| I-4 | Command palette shortcut | Important | Cmd-P |
| I-5 | Workspace structure: 3 crates | Important | Approve ADR-011 |
| I-6 | Behaviour on CLI version mismatch | Important | Warn, don't block |
| I-7 | Codex status precision: heuristics vs app-server | Important | Heuristics labelled honestly; ½-day app-server spike in M6 decides |
| D-1 … D-12 | See below | Can Be Deferred | — |

---

## Blocking (needed before M0 starts) — both resolved 2026-10-08 with the recommended defaults

### B-1 — GPUI source and component library

- **Context:** The official `gpui` crate on crates.io is a year stale (0.2.2). Current GPUI comes either from a Zed git revision or from `gpui-pre`, a weekly snapshot **published by Longbridge, not Zed**. `gpui-component` (Longbridge, Apache-2.0) provides the text input, palette, list, tabs and resizable split Tervy needs, and pins `gpui-pre` exactly. For a possibly-commercial product, this is a supply-chain choice you should make knowingly.
- **Options:**
  1. **gpui-component + gpui-pre (pinned).** Fastest path; depends on Longbridge's cadence.
  2. Zed git rev + hand-built widgets. Zed is the only upstream; adds weeks of widget work (text input with IME and selection is the expensive one).
  3. Zed git rev + vendored gpui-component pieces. Middle ground; we maintain the vendored code.
- **Recommendation:** **Option 1**, with exact pins, a committed `Cargo.lock`, deliberate upgrades between milestones, and option 3 as the documented fallback.
- **If deferred:** M0 cannot begin (the workspace's core dependency is undecided).

### B-2 — Toolchain installation on this Mac

- **Context:** Rust isn't installed. Only Command Line Tools are present. GPUI's default build needs full Xcode + the Metal toolchain component. The `runtime_shaders` feature *should* avoid that (unverified).
- **Options:**
  1. Install rustup now; try a CLT-only build; install Xcode + `MetalToolchain` only if that fails.
  2. Install Xcode up front (several GB, but certain to work, and needed later anyway for signing/notarisation).
- **Recommendation:** **Option 1.** A ½-day spike in M0.
- **If deferred:** No implementation can begin.

## Important (needed by the milestone shown; a default is assumed if you don't weigh in)

### I-1 — Default stop signal for processes (M3)

- **Context:** The first signal Tervy sends when stopping a process. Ctrl-C in a terminal sends SIGINT. Many tools treat SIGINT and SIGTERM the same; some don't (e.g. `docker compose up` stops containers gracefully on SIGINT).
- **Options:** SIGINT (Ctrl-C semantics) · SIGTERM (service-manager semantics).
- **Recommendation:** **SIGINT** by default, overridable per process (`stop_signal = "SIGTERM"` for Horizon, as in the config example). Escalation to SIGTERM and then SIGKILL is automatic either way.
- **If deferred:** SIGINT is used.

### I-2 — Default scrollback size (M1)

- **Context:** Scrollback memory is roughly lines × columns × ~24 B per cell (to be measured in M1). At 120 columns: 5,000 lines ≈ 14 MB and 10,000 lines ≈ 29 MB **per full terminal**. Twenty full terminals at 10k lines is about 580 MB.
- **Options:** 2,000 · **5,000** · 10,000 (Zed/Alacritty default) · per-kind defaults (e.g. processes 2,000, shells/agents 10,000).
- **Recommendation:** **5,000 globally**, configurable, then revisit with M1's real measurement. Agent TUIs mostly use the alternate screen, so their scrollback rarely fills.
- **If deferred:** 5,000.

### I-3 — Notifications when an agent needs approval (M4)

- **Context:** The status dot shows "needs approval", but you may be in another app.
- **Options:** None (dot only) · Dock badge count · Badge + macOS notification when the agent's pane isn't visible · Always notify.
- **Recommendation:** **Badge + a notification only when the pane isn't visible.** It's cheap and the main value of tracking many agents. Turned off with one setting.
- **If deferred:** Dock badge only.

### I-4 — Command palette shortcut (M2)

- **Context:** The obvious Cmd-K conflicts with the macOS terminal convention (Cmd-K = clear scrollback in Terminal.app and iTerm2). Cmd-Shift-P is VS Code's.
- **Options:** **Cmd-P** · Cmd-Shift-P · Cmd-K (giving up clear-scrollback).
- **Recommendation:** **Cmd-P** for the palette, keep Cmd-K for clear. Keybindings become configurable later.
- **If deferred:** Cmd-P.

### I-5 — Workspace structure (M0)

- **Context:** The brief suggested 7 crates. The plan proposes 3 (`tervy-term`, `tervy-core`, `tervy-app`) with module boundaries inside (ADR-011).
- **Recommendation:** Approve **3 crates**. Splitting a module into a crate later is mechanical.
- **If deferred:** 3 crates.

### I-6 — CLI version mismatch policy (M4)

- **Context:** `claude` and `codex` ship near-weekly. Tervy will know which versions it verified.
- **Options:** Block launching on untested versions · **Warn and continue** · Ignore.
- **Recommendation:** **Warn** (a banner with "run `scripts/cli-probe.sh`"). Blocking would make Tervy useless after every CLI update.
- **If deferred:** Warn.

### I-7 — Codex status precision (M6)

- **Context:** The adversarial review showed Codex's BEL/Enter heuristics can stick: a menu Enter never completes a turn, and BEL is untyped. Codex agents would show "Attention" / "Active (inferred)" rather than Claude's precise states. The app-server protocol has precise `thread/status/changed` (`waitingOnApproval`), but it's labelled experimental.
- **Options:**
  1. Heuristics only.
  2. App-server from the start.
  3. **A ½-day spike in M6 decides.**
- **Recommendation:** **Option 3.** Adopt app-server if the spike shows that Tervy still owns the lifecycle (its own server process, `--remote` TUI) and status is reliable.
- **If deferred:** Heuristics, labelled as inferred.

## Can Be Deferred (no MVP impact; trigger noted)

| ID | Question | Context and options | Recommendation | Consequence of deferring |
|---|---|---|---|---|
| D-1 | **When do we add a daemon?** | Daemon (processes outlive app) vs stay in-process. The supervisor's `Command`/`CoreEvent` seam makes it additive | Only if you repeatedly want servers running with the app closed. Consider Claude's own `--bg` supervisor for agents before building one | None. The seam exists |
| D-2 | **Git worktree per agent?** | Parallel agents on one repo can clobber each other. Both CLIs now have `--worktree` flags, which may make this cheap | Revisit after a month of daily use. Prefer the CLIs' own `--worktree` over Tervy-managed worktrees | Parallel agents share a directory (your current workflow) |
| D-3 | **Per-repo `.tervy.toml`** | Team-shareable process definitions. Needs a trust prompt showing exact commands, with trust pinned to a content hash | Design at commercial-phase start | Config stays personal and global |
| D-4 | **Keychain-backed secrets** | `env.KEY = { keychain = "svc/acct" }` resolved at spawn | Add when a process needs a secret outside `.env` | None now |
| D-5 | **Codex status via app-server** | Precise working/approval/input states; protocol is experimental | Adopt when it loses the experimental label or BEL proves noisy | Codex status stays heuristic |
| D-6 | **Persisted process logs** | Rotating size-capped logs so crash output survives restarts | Add only on demand | Output is lost when Tervy quits |
| D-7 | **Readiness probes** | "Running" vs "Ready" (port open / log regex) | After MVP, if dependent start ordering is needed | Start All starts everything at once |
| D-8 | **Read-only shell for Claude side conversations** | Sandbox `denyWrite` on the project + Bash allowlist (`git diff`, `git log`). Unverified | Investigate after M5; must pass the same enforcement tests | Claude side conversations can't run `git diff` (Codex's can) |
| D-9 | **Commercial readiness** | Trademark check on "Tervy", Developer ID signing, notarisation, auto-update, dependency audit | At the commercial decision | None for personal use (unsigned local build) |
| D-10 | **Linux / Windows** | GPUI now renders Linux via wgpu. `term::kill` and `core::env` hold the Unix/macOS specifics | Not before the macOS product is proven | macOS only |
| D-11 | **Multiple windows / richer layouts** | Brief advises against an elaborate window manager | One window, tabs + one split | None |
| D-12 | **Alternative terminal engine (libghostty)** | Promising, MIT, but Zig build dependency and pre-1.0 API | Re-evaluate in 2027 | None. alacritty_terminal is sound |
