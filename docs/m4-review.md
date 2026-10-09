# M4 review — Claude agents and restore

Status: **owner approved 2026-10-08**. The owner said “M4 looks good” and
authorized autonomous implementation, validation and commits through the remaining
phases while unavailable. M3 was committed as `9478a08`. Individual interactive
checks below are retained as coverage limits, not claimed as separately observed.

🧪 Tasks 4.1–4.9 are implemented. Each project has an Agents section with Launch
Claude, status dots and Stop/Resume. Cmd-Shift-R renames the selected tab. The
palette also offers Launch Claude, Resume other Claude session, and per-agent
Resume/Stop. Needs-approval agents contribute to the Dock badge; stopping/exited
agents do not. Primary agents use the official interactive CLI in a managed PTY.

🧪 SQLite schema 2 adds agent records and layout snapshots while preserving the
M3 session ledger. The supervisor owns the only connection. UI sends structural
layout changes; the actor captures shell cwd through `PROC_PIDVNODEPATHINFO` with
birth-time checks and debounces writes for 250 ms. Quit captures cwd again and
flushes the pending snapshot. Restore preserves project, tab order, names,
selection and expansion through stable keys. Shells start fresh PTYs at their
saved cwd, falling back to the project when a directory is missing. Agent tabs
restore as Resume stubs; no agent resumes automatically. Side records are excluded
from automatic restore. Terminal output and hook tokens are never persisted.

🧪 The provider builds complete New/Resume/ResumePicker argv on a spawn worker,
using the cached login/interactive environment. Installed-version checks have a
2-second timeout and warn on drift from 2.1.295. Every launch derives its profile
from the stored role. Golden tests prove a Side record cannot produce Primary
argv; Side New/Picker are rejected until M5 defines their lifecycle. An agent's
PTY exit remains authoritative. Resuming waits for the previous session to be
empty and its reader joined; surviving ledger entries prevent duplicate starts.

🧪 The hook helper dispatches before logging or GPUI. Claude stdin is streamed
through a bounded buffered parser, retaining only status fields; Codex last-argv
transport is present for M6. Stdin has a 200 ms deadline and 64 MiB ceiling;
connect/write share a separate 200 ms deadline. All errors return zero with empty
stdout. No prompts, tool arguments/results or transcript paths enter IPC.
The Unix listener blocks at idle and permits at most 16 readers, each with an
absolute 250 ms deadline and a 64 KiB message cap. Its queue is bounded at 128;
the actor checks identity, launch generation and token, and limits each agent
to 100 accepted events per second. Directory/socket permissions are 0700/0600.
Long support paths use an owned private short socket directory under `/tmp`.

✅ [Claude's documented PermissionRequest](https://code.claude.com/docs/en/hooks#permissionrequest)
lacks `tool_use_id`; matching falls back to tool name. Two concurrent requests
for the same tool are ambiguous. PermissionDenied covers auto mode; manual
failure/denial is handled through PostToolUseFailure. Another Stop hook can block
a candidate idle transition, corrected by later events. Status is best-effort.
🧪 Scenario tests cover failure after approval, denial, API failure, parallel
requests for distinct tools and continuation after a blocked Stop. These are
synthetic state-machine tests; real interactive scenarios still need review.

🧪 `./scripts/check.sh` passes fmt, Clippy with warnings denied, **55 tests** and
the license allowlist. The ignored crash helper is explicitly invoked by its
parent integration test. New coverage includes schema-1 migration, actual shell
cwd/restore, no automatic agent/Side launch, wrong-token and stale-launch rejection,
rate limiting, slow/oversized clients, helper large-field projection, held-open
stdin deadlines, full provider argv and role-derived permissions. AppKit badge
access stays on the UI thread. No dependency pins or license exceptions changed.

🧪 `scripts/cli-probe.sh` verifies A5 with temporary user-level and injected Setup
hooks under `--init-only`, without a model call or changing real user settings.
The authorized `--real` Haiku probes passed: a running-parent fork inherited
BLUEHERON and acquired a distinct hook session ID, its headless canary files
remained unchanged, the parent completed, and resume preserved identity/context.
The first paid attempt exited before Bash sleep for an uncaptured reason; the
retry passed. Failure reports now retain documented CLI output diagnostics.
No transcripts/rollouts were read. Interactive enforcement remains M5's first gate.

🧪 Measurements are in [perf/m4.json](../perf/m4.json), on the M2 Max replacement
machine. Final release helper p50 **5.94 ms**, p95 **6.78 ms**, maximum **7.65 ms**
over 100 measured runs after five warmups: warmed timing meets 10 ms. First helper
invocation **437.25 ms** misses that target. Hyperfine was unavailable, so the
script uses Python wall time around exec, stdin, real socket delivery and wait.
Ten isolated-home release app launches measured first-frame p50 **167.85 ms**,
first/p95 **659.2 ms**; first launch misses 500 ms. Startup samples precede the
final buffered-reader/status-dot polish. The approved baseline is retained;
A11 and the M3 stress limitations remain open, with no new idle/stress pass claimed.

🧪 Owner review reported that Tab did not accept Claude's muted suggestion.
Terminal-scoped `NoAction` bindings passed keymap precedence tests, but the owner
confirmed that the behavior still failed. The revised build uses explicit
terminal Tab/Backtab actions, which encode through the negotiated protocol and
stop action propagation. Keymap tests verify their priority over root navigation
and leave input fields alone; encoder tests cover legacy/kitty Tab and Backtab.
🧪 Owner confirmed the revised build still does not accept a suggestion with
Tab, but Right Arrow does, and accepted that workaround. Tab acceptance remains
an unresolved limitation; it is not claimed as fixed. Opt-in `DEVDESK_TRACE_TAB` diagnostics
report only Tab action resolution, context, mode and encoded bytes; they do not
record ordinary input or terminal output.

🧪 At the owner's request, projects, configured processes and agents now have
sidebar × removal controls with an explicit Remove confirmation. Closing a tab
still preserves a process/agent identity. Removing an agent deletes its DevDesk
record and stops its owned session; it does not delete Claude's own conversation.
Removing a process edits the global TOML definition and stops that instance.
Removing a project deletes its DevDesk config and agent records, and stops its
managed sessions; files in the project folder are untouched. Config writes retain
unrelated tables/comments and use atomic replacement with an editor-race check.
Late hooks cannot recreate a record while deletion is stopping its PTY.
Integration tests verify real process cleanup, project-shell cleanup, unrelated
config/agent preservation, and no resurrection on relaunch. ❓ Visual deletion
acceptance remains pending in the rebuilt app.

Owner acceptance, using the existing ai-crm config (process autostart stays off):

1. Launch Claude, submit a harmless prompt, and observe Idle → Working → Idle.
   Trigger a permission request and check Needs approval plus the Dock badge,
   then approve or deny and confirm it clears. Check failed-tool/API/blocked-Stop
   scenarios when a safe fixture is available.
2. Rename the agent and arrange several shell/agent/process tabs. Change a shell
   cwd, quit, then `cargo run -p devdesk-app`. Confirm order, selection, names and
   cwd; the agent should be a stub. Resume and ask about the earlier prompt to
   confirm the same conversation in the interactive TUI.
3. Cmd-P → Resume other Claude session, choose a session in the CLI picker, stop
   it and resume through DevDesk. Check that the chosen session is tracked.
4. Stop/close Claude and confirm the sidebar identity remains resumable. Kill
   DevDesk only with a safe synthetic agent if checking crash behavior; the helper
   must return without hanging when the app is gone. Existing synthetic helper
   and session-recovery tests cover bounded errors and orphan cleanup separately.

🧪 Owner approved M4 as a milestone. CLI headless success and synthetic tests
do not independently establish every interactive scenario listed above. The Tab
workaround and recorded performance limitations remain accepted open issues.
