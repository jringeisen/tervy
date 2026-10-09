# M3 review — configured processes

Status: **owner approved 2026-10-08; committing at owner request**. M1 and M2 were committed as
`dbbc689` at the owner's request. The owner confirmed the process-tab close fix and requested committing M3, then continuing to M4. Performance limitations below remain open.

🧪 Tasks 3.1–3.6 are implemented. Configured processes have persistent rows and
terminal tabs, Start/Stop/Restart controls, palette actions, Start All / Stop All,
once-per-run project autostart and a config-change marker.
✅ Closing a process tab (× or Cmd-W) requests Stop and dismisses the tab;
its sidebar row and history remain. Clicking that row reopens the terminal,
including while it is stopping. Closing a shell still waits for its existing
foreground-job confirmation and cleanup. Commands use the
resolved login/interactive environment with `$SHELL -c`; `shell = "interactive"`
opts into `$SHELL -i -c`. Relative `cwd` values are relative to the project;
per-process environment overrides are applied to a copy of the resolved env.

🧪 The supervisor owns lifecycle state. Unexpected leader exit enters Stopping
until the whole owned session is empty and its reader has drained and joined.
Process cleanup uses its launch-time stop signal and timeout, then SIGTERM and
SIGKILL three seconds later. Default policy is SIGINT / 10 seconds / 3 seconds.
Repeated stop requests do not extend an active deadline. Quit still stops all
sessions in parallel with the app-wide 15-second deadline, including Horizon's
longer configured grace period.

🧪 A configured process keeps one grid for this app run. Restart preserves its
history, copies crashed alternate-screen text into primary history, resets modes
through Handler methods, and adds a `restarted after exit N` separator. Each
instance has a fresh PTY, parser, input channel, event sink and launch generation.
The old reader is joined on a worker before attaching the new reader. The small
Apache-2.0 terminal event-sink setter is attributed in [vendor/README.md](../vendor/README.md).

🧪 On-failure restarts back off from 1 to 30 seconds, respect `max_retries`, and
give up after five failures in 60 seconds. Manual Start resets the crash counter;
Stop cancels queued retries. Hot reload preserves the current launch's settings
until restart. Removing a definition preserves its existing history but prevents
new starts. Infrastructure failures require manual intervention.

🧪 `state.db` uses bundled SQLite, WAL, FULL synchronous writes, a 250 ms busy
timeout and schema version 1. The supervisor alone owns the connection. The
ledger records SID, leader birth time, launch generation and owner before
Running. Relaunch offers Leave / Stop for verified survivors, including children
without their leader. Stop sends SIGTERM, escalates to SIGKILL after three
seconds, and removes records only after verified emptiness. Leave retains the
record and blocks a duplicate start of that configured process; the palette's
Recover Previous Sessions action offers it again. Recovery pauses automatic
retries. Self-`setsid` daemons remain outside scope; macOS lacks an atomic
birth-time-and-kill syscall. Scrollback is in memory and does not survive an app
crash. Recovery polls while surviving processes remain; app quit remains bounded.

🧪 `./scripts/check.sh` passes formatting, Clippy with warnings denied,
**42 tests**, and the license allowlist. One ignored test is a helper subprocess
that the crash-recovery integration test explicitly runs. Coverage includes:

- State transitions, monotonic backoff, crash-loop give-up and configured stop
  deadlines with a fake clock.
- A crashed alternate-screen TUI with mouse/kitty modes; restart preserves the
  same grid, restores default modes and routes terminal replies to the new PTY.
- A leaderless, SIGINT/SIGTERM-ignoring child holding a TCP port; restart waits
  for it to die before binding the same port again.
- SIGKILL of a separate supervisor process, discovery on relaunch, and cleanup
  of its leaderless surviving child from the persisted ledger.
- Automatic retries retaining history, Stop cancelling retries, first-open
  autostart, idempotent Start All / Stop All, and launch-time config snapshots.
- Schema migration/newer-version refusal and stale birth-time records; existing
  session-sweep tests cover unrelated reused identities and background groups.

The owner's `ai-crm` global config contains `php artisan serve`,
`php artisan horizon`, and `npm run dev`, all with **autostart off**. Horizon uses
SIGTERM with a 30-second grace period and on-failure restart, up to five retries.
The config backup is `/private/tmp/devdesk-ai-crm-config-before-m3.toml`.

Owner acceptance still needs the running project:

1. Cmd-P → Start All Processes. Confirm Laravel, Horizon and Vite start and their
   output is readable; `npm run dev` must find the nvm environment.
2. Cmd-P → Restart Horizon. Confirm it stops, restarts and retains earlier output.
   Restart Laravel and confirm it rebinds without an address-in-use error.
3. Stop All Processes, then start individual rows and check the status changes.
4. Optional destructive review in a safe project: kill Horizon externally and
   inspect automatic restart/history. Synthetic crash/port/recovery fixtures
   already exercise those lifecycle paths without running application jobs.
5. Interactive process input (`php artisan tinker` configured temporarily) remains
   a manual acceptance check. Shell input was reviewed in M1/M2.

❓ The real Laravel/Horizon/Vite behavior and visual acceptance are pending owner
review. Automated fixtures do not establish the health of the owner's app,
Redis connection, job workload or Vite configuration. A11 remains unresolved.

🧪 Release probes are recorded separately in [perf/m3.json](../perf/m3.json);
the approved M2 baseline is retained. Normal isolated-home first-frame median
was **188.9 ms**, first/p95 **719.8 ms**. The comparable in-memory terminal
median was **252.2 ms**, up **53.5%** from M2's 164.35 ms; the cause is unverified.
Painted echo p95 was **8.54 ms**. Startup medians and echo meet their budgets,
but first-launch samples exceed 500 ms. Both eleven-session stress trials
**failed**: one had no frame samples, the repeat timed out after 30 seconds.
All 22 traced sessions were verified empty after cleanup. Sampling found the
main thread waiting in AppKit and the supervisor waiting for messages, rather
than blocked on a terminal mutex. ❓ A window/display condition is suspected;
a visible-window stress rerun and the startup regression investigation remain
pending. These probes are not a stress-performance pass or an A11 fix.

🧪 Owner reported the process workflow looked good apart from closing process tabs, then confirmed that fix (“looks good now”) and requested “commit and continue”. Individual manual acceptance checks above are not claimed as separately verified; startup/stress limitations remain recorded.
