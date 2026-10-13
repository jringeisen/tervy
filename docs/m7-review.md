# M7 — Hardening and performance review

🧪 Completed under the owner's autonomous implementation, validation and commit
authorization on the replacement Apple Silicon Mac (Mac14,6, M2 Max, 96 GB,
macOS 27.0.1). Synthetic projects and workers only; no owner services or model
prompts are used by the M7 gate. M5/M6's real-provider probes are recorded in
their reviews. The absent owner's visual review is not claimed.

| Task | Implementation and evidence |
|---|---|
| 7.1 noisy terminals / A8 | 🧪 Twenty `yes` readers plus one quiet echo child, three visible panes, ten project switches. Frame p95 **9.959 ms**, echo p95 **9.590 ms**, palette **10.315 ms**, shutdown **124.741 ms**. All 21 sessions empty afterward. Stable app thread count **31** under load. Retain one reader per PTY: no shared reader is needed by this measured workload. This does not establish the untested 40-terminal case. |
| 7.2 memory / A10 | 🧪 Separate empty-window, single-shell, 20-idle-shell and 20-full-history trials are in `perf/baseline.json`. Grid dimensions and every 5,000-line cap are asserted before sampling. Default history remains **5,000 lines**: full histories are user-controlled memory, reported separately from empty-terminal overhead. |
| 7.3 idle / A11 | 🧪 Windows are verified on screen/frontmost before and after sampling; `caffeinate` prevents display sleep. The 20-idle trial settles ten seconds, then measures sixty. Source inspection finds no recurring Tervy deadline for idle shells/agents without jobs. A single benchmark-only grid report fires at five seconds, before the interval. `sudo -n powermetrics` fails because a password is required; process counters do not prove zero timer wakeups or attribute upstream callbacks. The visible CPU budget remains missed. |
| 7.4 panic / recovery | 🧪 Real subprocess test installs the production hook, launches a worker and child, panics, and verifies exit 101, source-location-only log and empty owned session. SIGKILL/relaunch tests verify generic leaderless-session recovery and a separately journaled Codex job after its TUI disappears. Panic logging uses a separate capped append descriptor, not the tracing mutex; the hook sends Shutdown without waiting. |
| 7.5 app bundle | 🧪 `scripts/bundle.sh` builds `target/Tervy.app`, validates `Info.plist`, and generates an original icon with Apple drawing tools. Native split/keyboard probes run from its actual executable path. No distribution signing or notarisation; the executable may have the linker's ordinary ad-hoc Mach-O signature. Hook paths derive from `current_exe`. |
| 7.6 baseline / comparison | 🧪 M2 is preserved in `perf/baseline-m2.json`; M7 becomes `perf/baseline.json`. `scripts/perf/compare.py` reports >15% regressions, requires an explicit recorded reason, rejects missing metrics, and marks different workloads/visibility controls incomparable. No failed metric is converted to zero. |
| 7.7 keyboard | 🧪 Native Cmd-P/Enter routes switch primary/side focus. Palette routes expose project/process/agent removal, tab/history reopening, split width, links, launch/resume/stop/restart, config reload, recovery and quit. Existing destructive confirmations remain in place. Shift-PageUp/Down and Cmd-Home/End scroll history; Cmd-A selects it for Cmd-C. `README.md` lists shortcuts. CLI-specific TUI navigation remains the CLI's behavior. |

🧪 `scripts/check.sh`: **69 tests pass**, fmt, Clippy `-D warnings` and license
allowlist pass. The upstream `block 0.1.6` future-incompatibility notice remains;
it is not a workspace warning failure. Shell and Python scripts parse. Native
split, real shortcut dispatch and resize probes are separate from headless tests.

## Concurrent creation fix

🧪 The full-history fixture initially lost two or three terminals before launch:
parallel `openpty` calls returned raw error **-6**. A barrier-based test reproduced
the error inside PTY allocation, before child creation. Tervy now serializes
only the short allocate/spawn stage on workers; no Term lock or reader exists
when that guard is held. Input, readers, painting and teardown do not acquire it.
Twenty simultaneous launchers start, produce ready markers and cleanly tear down.
The original full-history fixture now retains all twenty workers and every cap.
❓ The exact libc/kernel cause is not established. A speculative birth-identity
retry was removed after stage diagnostics disproved that hypothesis.

## Memory change

🧪 Before the change, a visible 20-idle-shell trial used **108.79 MB** in Tervy
and **103.74 MB** in its separate shell processes. The pinned Apache-licensed
Alacritty event loop used a **1 MiB** read buffer on each reader's stack. Its
existing locked-read batch is already approximately 64 KiB. Reducing the read
buffer to **64 KiB** saves committed per-reader memory; the attributed source
change and registry diff are in `vendor/README.md` and `vendor/tervy.patch`.
It introduces no new license exception or dependency version.

🧪 The smaller buffer passes the 20-noisy probe and the printable **100 decimal
MB** flood: queued input is preserved, **99** frames paint, maximum measured
paint gap **10.200 ms**, and end-to-end probe completion **1,330.039 ms**. The
probe includes startup and input echo; this is not an isolated parser-throughput
measurement or a fresh comparison against a standalone Alacritty app.

🧪 Final visible footprints: empty **64.65 MB** (misses 60 MB); single shell
**69.57 MB**; twenty empty-history shells **89.21 MB** (passes 100 MB); twenty
full histories **375.56 MB**. Separate child footprints are **0 / 4.90 / 103.77 /
128.34 MB** respectively. Full-history grids are nineteen 100-column panes plus
one 113-column pane; their inline history-cell floor is **241.56 MB**, excluding
visible rows, alternate grids, metadata and allocator overhead. Keep the default
history cap; reducing it to hide empty-window overhead would not address that
budget. Twenty-shell CPU is **0.714%**, with **0.100 package-idle wakes/s** and
**120.47 interrupt wakes/s**. Empty/single CPU is **0.687% / 0.708%**. All visible
CPU trials miss 0.3%; counters do not identify timer origins. See the JSON for
exact values and timing intervals.

## Crash limits

🧪 Codex's separate job sessions are journaled once a second while a turn is
inferred active or known jobs remain. Already persisted identities are checked
without repeated WAL writes; failed inserts are retried. Finished identities
are pruned only after verified emptiness. No scanning deadline exists for idle
agents without jobs. An idle agent is not scanned merely because another agent
is active. Store writes remain on the actor; kernel discovery stays off GPUI.

❓ A process born and orphaned between discovery scans can escape capture. The
OS has no atomic “create descendant and persist its identity” barrier exposed
by this integration. Unknown deliberate daemons are never killed by name. The
journal improves hard-crash recovery; it does not guarantee ownership of every
possible detached process. The tested recovery explicitly destroys the fixture
TUI before relaunch so rediscovering a live parent cannot hide this limitation.

🧪 Ordinary Rust unwinding allows graceful supervisor cleanup. A panic inside
a foreign AppKit callback can abort before cleanup completes; an early native
smoke assertion demonstrated this while still emitting the source-location log.
The final native probe passes. SIGKILL and abort rely on SIGHUP plus the journal
on relaunch. Logs stop growing at 5 MiB each and rotate on a later launch;
terminal output and panic payloads are not written to these logs.

## Performance limits

🧪 First-frame median **173.3 ms** meets 500 ms and is about **5.45%** above M2.
First sample/p95 **633.9 ms** misses 500 ms. This trial follows other native
probes; it is not a true cold-cache run or the first app invocation after build.
Neither shader caches nor OS caches were flushed. Single-shell painted echo p95
**7.853 ms** and PTY-write sample p99 **1.009 ms** meet their 16/2 ms budgets.

🧪 GPUI's upstream display-link callback continues with an idle visible window
(A11). No public pause API exists in the pinned sources. The prior investigation
and public-API limitation are retained in technical-research §10c; no unsafe
AppKit workaround, dependency upgrade or GPU backend fork is introduced. Low
M2 CPU values had uncontrolled visibility and cannot demonstrate a regression
or fix against this explicitly visible trial. Empty-window memory and visible
CPU misses are reported as misses, not milestone acceptance successes.

Reproduce with `scripts/perf/run-m7.sh`, leaving benchmark windows untouched.
The baseline records absolute values, context, process trees and cleanup results.
