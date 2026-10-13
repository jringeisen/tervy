# M2 review — projects, tabs and shell lifecycle

Status: **owner approved 2026-10-08; committed with M1**. M1 was approved on
2026-10-08. 🧪 Owner reviewed the running M2 window and said “Looks good!”. M3 has not started.

🧪 Tasks 2.1–2.8 are implemented: global TOML configuration and hot reload;
comment-preserving Add Project folder picker; a synchronous supervisor actor;
macOS session sweeps and escalation; project sidebar and terminal tabs;
foreground-job close confirmation; fuzzy command palette and keyboard actions;
single-instance activation, hide-on-close and confirmed parallel quit.

🧪 The supervisor owns each `TerminalSession`. Views retain cheap input/grid
handles and their terminal entities while hidden. Output bypasses the actor;
only lifecycle messages enter its mailbox. A launch generation rejects stale
exit events and close approvals. Hidden panes consume updates without requesting
redraws, and activation invalidates the selected grid. No idle actor timer is
armed; debounce and stop timers exist only while work is pending.

🧪 Stopping sends SIGHUP, SIGTERM at two seconds and SIGKILL at five seconds,
then verifies the **whole session** is empty and the reader has drained before
closing or restarting. The macOS SDK's process list and birth times cover
background process groups and surviving children whose leader has exited.
Identity is checked again before signalling. macOS has no atomic
birth-time-and-kill syscall, so a final check/syscall race remains. Processes
that create a new session with `setsid` are outside the agreed scope. The
15-second quit deadline reports an unclean result if it cannot verify cleanup;
force-quit recovery and the persistent ledger belong to M3.

🧪 `./scripts/check.sh` passes formatting, Clippy with warnings denied,
**32 tests** and the license allowlist. New coverage includes fake-clock stop
deadlines, stale lifecycle events, stop-during-spawn, current-launch foreground
confirmation, last-good configuration, sub-second reload, comment preservation,
single-instance activation, leaderless children and SIGHUP/SIGTERM-ignoring jobs.
The busy-shell test exercises parallel forced shutdown of ten real PTYs.

The global file is `~/Library/Application Support/Tervy/config.toml`.
Add Project creates the file if necessary and appends a project table. Manual
edits update the project list after a 120 ms debounce; invalid edits show a
banner and preserve the last good configuration. Removing a project keeps its
existing tabs reachable until closed. Terminal font/scrollback settings apply
to new tabs. Process and agent configuration is parsed for schema compatibility;
launching those records belongs to later milestones. Terminal layout/renames
are currently in memory; restore is M4.

```toml
[settings]
scrollback_lines = 5000
font_family = "Menlo"
font_size = 14
option_as_meta = false

[[projects]]
id = "tervy"
name = "Tervy"
path = "~/www/tervy"
```

| Action | Shortcut |
|---|---|
| Command palette | Cmd-P |
| Add Project | Cmd-O |
| New terminal | Cmd-T |
| Close terminal / dismiss overlay | Cmd-W |
| Rename terminal | Cmd-Shift-R; Enter saves, Escape cancels |
| Select tab | Cmd-1…9 |
| Previous / next tab | Cmd-Shift-[ / ] |
| Clear scrollback | Cmd-K |
| Quit with confirmation | Cmd-Q |

❓ Owner review in the running window:

- Add three project folders, make several tabs, rename one, and switch with the
  sidebar, palette and shortcuts. A command left running must continue across
  switches. Search `swpj` in the palette to exercise noncontiguous fuzzy search.
- Edit a project name in the global TOML; check the sidebar changes. Temporarily
  enter invalid TOML and verify the banner, then undo it.
- Run `vim` or `sleep 1000` and close its tab: Cancel preserves the job; Stop
  closes it. Run two background sleeps in another tab and close it too.
- Click the red window button: Tervy hides and jobs continue. Launch Tervy
  again to bring the same window back.
- Run `exit 7`; check the exited state and Return to restart. Check palette and
  rename focus, Escape, and the existing terminal input/clipboard behavior.
- Cmd-Q: Cancel returns to the terminal; Quit shows progress and stops all jobs.

🧪 Performance results and their sampling limits are in
[baseline.json](../perf/baseline.json), with raw multi-session samples in
[m2-many.json](../perf/m2-many.json). Reproduce with `./scripts/perf/run-m2.sh`.
The stress-only three-pane arrangement is instrumentation; normal split-pane
controls belong to M5. The probe uses ten `yes` producers plus one quiet echo
child across three projects, twenty synthetic keystrokes, ten switches and
quit during output. It records managed children separately from Tervy; no
agent/model call is made. Timings end at GPUI frame/paint callbacks and exclude
physical keyboard delivery and compositor scanout.

🧪 Final release results: first-frame p50 **164.3 ms** (first/p95 **619.4 ms**);
single-terminal echo p95 **8.15 ms**, PTY-write p99 **1.620 ms**. Under ten
noisy terminals, frame p95 **9.30 ms** (maximum **39.14 ms**), maximum project
switch **10.90 ms**, palette **8.29 ms**, quiet echo p95 **8.94 ms**, and
shutdown **108.08 ms**, with **zero survivors** across all eleven owned sessions.
The first launch spent **402.8 ms before main**; its p95 regresses **23.3%**
from M1 while the median is unchanged. Sequential launches do not establish
true cold-cache performance.

🧪 A clean one-shell idle repeat measured **70.06 MB** (+7.0% from M1),
**0.0073% CPU**, no package-idle wakes and **0.383 interrupt wakes/s**, after
a ten-second settle over sixty seconds. Window visibility/occlusion was not
controlled; this low-wakeup state does **not** prove A11 fixed for a visible
window. An earlier trial had five shell launches, **84.85 MB / 1.50% CPU**,
and is retained as nonbaseline evidence rather than a one-shell idle result.
A diagnostic sample shows the supervisor and PTY reader waiting normally.
The twenty-idle-shell memory/CPU budgets remain M7 work.

🧪 The normal debug app is open for review. A second executable invocation
exited successfully in **8 ms** and retained the original app process; the
activation socket brings that existing window forward. All seven recorded
idle-trial sessions were independently checked afterward: zero survivors.
