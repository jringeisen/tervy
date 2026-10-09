# M1 review — one great terminal

Status: **owner approved 2026-10-08; committed with M2**.
🧪 Owner reviewed the running window and said “looks good lets continue”; M2 is authorized. Measurements are in [perf/baseline-m1.json](../perf/baseline-m1.json);
M0's replacement-machine record is preserved in [baseline-m0.json](../perf/baseline-m0.json).

🧪 Release first frame p50 **164.1 ms** (first/p95 **502.4 ms**); synthetic
key-to-painted-echo p95 **9.21 ms**. The idle shell uses **65.49 MB**, but CPU
**0.758%** misses the **0.3%** budget. A11 remains documented upstream work.

🧪 `scripts/check.sh` passes: formatting, Clippy with warnings denied, **23 tests**,
and the license allowlist. The tests cover actual PTY output draining, direct
signal inheritance, process-group SIGINT, immediate CPR/DA/kitty replies,
recorded agent startup fixtures, key/mouse encoders, snapshot batching, the
snapshot/wakeup race, UTF-16 boundaries, clipboard read denial and adversarial
metadata bounds. `cargo run --release -p devdesk-core --example env_probe` finds
this machine's nvm `npm` and Herd `php` with the actual cached resolver.

🧪 Implemented tasks 1.1–1.16: explicit login-shell PTY; background, cached env
resolution; coalesced redraws; deferred color/size queries; snapshot rendering;
font metrics and fallback; grid-anchored Unicode; legacy/kitty keys; per-frame
resize; 5,000-line scrollback; selection, copy and bracketed paste; exit/restart
and bell; native marked/committed text; mouse/focus reports; offline fixtures.
The encoder is an original implementation from the published protocols.
The scrollback limit is a spawn parameter; M2 will connect it to TOML config.

🧪 Ownership: the M1 view temporarily owns one `TerminalSession`. Its clones
carry input and the shared terminal grid. A PTY thread parses into that grid;
the UI clears dirty **before** copying the visible snapshot, then shapes and
paints after releasing the lock. Only startup uses `OnceLock` barriers; the
resolved environment is immutable afterward. Teardown syscalls and joining
run on a worker. M2 transfers session ownership to its supervisor and adds
complete session sweeps, escalation, close/hide and quit confirmation.

🧪 New evidence required small patches to the pinned Apache/MIT dependencies:
unbounded combining marks, links and OSC payloads bypassed the scrollback cap.
Limits, attribution and the exact registry diff are in
[vendor/README.md](../vendor/README.md) and [vendor/devdesk.patch](../vendor/devdesk.patch).
Versions remain alacritty_terminal 0.26.0 and vte 0.15.0. No new license exception.

🧪 Owner already verified basic typing and Option-e then e. The reported gap
before é was fixed by retaining the font's fractional cell advance. This
machine's startup `compinit` error came from a dangling Homebrew completion
symlink; it was reversibly renamed to
`/opt/homebrew/share/zsh/site-functions/.devdesk-disabled-brew-services`.
Fresh login shells start cleanly. htop 3.5.3 and fzf 0.74.4 were installed for
review; Homebrew's automatic cleanup reported a cache permission error, while
both installed executables work.

Owner approved M1 after reviewing the running window. The original visual checklist follows; individual IME/font configurations were not separately recorded:

- Run `vim`, `htop`, `less docs/implementation-plan.md`, `git --paginate log`, and
  `printf 'one\ntwo\n' | fzf`. Move around, select items, and resize the window.
- Drag-select; double-click a word; triple-click a line. Check Cmd-C and Cmd-V,
  including multi-line paste. Shift bypasses mouse reporting for selection.
- Type é again, emoji and CJK; try an IME composition if configured. Check box
  drawing and the existing prompt. `DEVDESK_FONT_FAMILY` can select an installed
  Nerd Font; Menlo is the default, with CoreText fallback and cell-drawn common
  box/powerline separators. `DEVDESK_OPTION_AS_META=1` opts out of Option accents.
- Start `claude`, then `codex --no-daemon`. Check navigation, resize and
  Shift+Enter. Paste multiple lines into the agent input and confirm it remains
  an unsubmitted draft. The captured fixtures submit no model prompt.
- Run `exit 7`; confirm its status stays visible and Return opens a fresh shell.
  `printf '\a'` should show the bell indicator. Check light/dark appearance.

🧪 The benchmark harness exercises real GPUI dispatch/native input, 100 MB of
printable `cat` output with queued input, and `seq 1 2000000`. It reports the
view-init and PTY-spawn timing to first content paint for each CLI, without claiming authentication/readiness
or validating every interactive screen. Physical keyboard delivery and display
scanout are excluded from the latency samples. Agent subprocess memory is not
included in DevDesk's own footprint measurements. The separate CLI tree sample
includes configured child processes and covers startup only; it is not a steady
agent memory budget.

❓ A11's pinned GPUI backend still has no public display-link stop/rearm API.
Empty-window controls exceeded the original idle budgets; later shell samples
vary with system/display conditions. No GPUI fork or idle polling/blink timer
was added. Refresh-rate attribution and 20-session budgets remain M7 work.
The pinned GPUI keystroke API also omits physical keypad/base-layout metadata;
the engine's keypad/alternate-key encoders are tested, but those distinctions
cannot all be supplied by the current macOS event adapter.

Reproduce: `./scripts/check.sh`, then `./scripts/perf/run.sh`. Results go to
ignored `perf/local/` for review. This opens and closes benchmark windows and
starts the installed agent CLIs without submitting prompts. Close a separate
review window first for an isolated run. The full read-only/fork CLI probes
remain prerequisites for their integration milestones: Codex is now 0.162.0;
M1's terminal protocol capture does not re-verify those flags.
