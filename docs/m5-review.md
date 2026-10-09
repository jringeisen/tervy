# M5 review — Claude side conversations

Status: **implemented and autonomously validated**, under the owner's 2026-10-08 authorization to finish, validate and commit the remaining milestones while unavailable.

🧪 Cmd-Shift-A and “Ask Agent Question…” open a one-line question input for a running primary agent after its initial hook. The supervisor persists a Side record before spawning; its cwd and stable parent identity come from the parent record. The complete Fork argv includes the observer preamble and hardened read/search-only flags. Only a distinct `SessionStart(source=fork)` supplies the resumable fork ID; an unexpected source or parent ID stops the side.

🧪 The right pane uses the pinned gpui-component resizable widget through gpui-kit. Its chrome identifies read-only mode, local snapshot time and the current parent name. Esc or Cmd-W with side focus, and the pane’s close button, stop/dismiss only the side. Replacing a displayed side stops the previous side. Sidebar history is nested under the parent; Resume targets the saved fork ID and reapplies all read-only flags. Side history restores closed, never automatically launching. Removing a parent removes/stops its side history as well. Existing M4 JSON records remain compatible through defaulted optional fields; no SQLite migration is needed.

🧪 The paid, ignored test uses real devdesk-term PTYs and synthetic fixture data only. Claude 2.1.295 with OAuth/Haiku passed five interactive forks while a bounded Python parent task waited at least 120 seconds and then completed. All five retained BLUEHERON and the supplied design reason, refused Write/Edit/Bash, left fixture canaries unchanged, and received distinct fork IDs. Answers expressed uncertainty about live parent status without falsely saying it stopped. Each side and the parent were fully cleaned up. No transcript or rollout files were read.

🧪 Initial probe failures were diagnosed and corrected: Claude rejected standalone sleep before any fork; directory trust needed separately timed Down/Enter events; and the first assertion incorrectly rejected an honest “unknown” live status. The final criterion follows the acceptance list. Only each probe’s own newly created directory is trusted automatically.

🧪 A zero-model synthetic CLI integration exercises the production provider, hook socket and supervisor with real PTYs: persisted parent/cwd/fork identity; side-only close with parent alive; hardened same-fork resume; and cascading parent removal with no survivors. Its initial failures were test-harness socket framing, stop-status expectations, and reading the argv file before the resumed CLI’s hook. The corrected test passes.

🧪 A native release render probe verifies 480/480 px split geometry, a retained 960 px parent after side close, history reopening and question-input access. This is geometry/state evidence, not a screenshot or human visual review.

🧪 `./scripts/check.sh` passes formatting, Clippy with warnings denied, **57 tests**, and the license allowlist. The paid CLI probe remains ignored in ordinary checks. No dependency pins or license exceptions changed. Release first-frame startup p50 is **169.0 ms**, first/p95 **589.9 ms** over ten runs: median meets 500 ms and changes +0.69% from M4; first post-build launch still misses the target. Measurements: [perf/m5.json](../perf/m5.json).

📘 Production adds `ShareOnboardingGuide` explicitly to `--disallowedTools`; 🧪 interactive argv accepts it and no account action was invoked. ❓ There is no independently enumerated interactive tool registry through a documented interface, so future account tools remain a coverage limit. The documented hook projection does not capture the parent model; third-party provider/auth and model inheritance under restricted settings remain unverified. OAuth works here.

Reproduce:

- `./scripts/check.sh`
- `cargo test -p devdesk-core --test claude_interactive -- --ignored --nocapture` (paid synthetic Haiku calls)
- `DEVDESK_PERF_LAYOUT=side target/release/devdesk` (native synthetic render only)
- `DEVDESK_PERF_LAYOUT=normal ./scripts/perf/startup.sh release 10`

M7 still owns visible idle/wakeup investigation and the 20-terminal stress gates. M4’s accepted Right Arrow suggestion-acceptance workaround remains unchanged; Tab acceptance is not claimed fixed.
