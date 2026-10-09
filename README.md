# DevDesk

Native macOS developer workspace for Apple Silicon: shell terminals, configured
Laravel/Vite processes, embedded Claude Code and Codex TUIs, and read-only side
conversations made with the CLIs' native forks. Implementation milestones M0–M7
are complete; performance limitations and validation evidence are recorded in
[the final review](docs/m7-review.md).

Build with Rust via rustup and Apple's Command Line Tools. Install `cargo-deny`
with Homebrew for the quality gate. Use Xcode only if your toolchain cannot
compile the required Metal shaders. Dependencies are deliberately pinned.

```sh
./scripts/check.sh
cargo run
./scripts/bundle.sh
# Optional separate destination for reviewing a build:
# ./scripts/bundle.sh target/DevDesk-review.app
```

The last command creates `target/DevDesk.app` with an original icon and
`Info.plist`, without distribution signing or notarisation. Move the app to its
final location **before launching agents**: each CLI receives the absolute path
to that bundle's `Contents/MacOS/devdesk` hook helper. Stop agents before moving
or replacing the bundle, then resume them from the new app.

The whole workspace follows macOS light/dark appearance, including open terminal
panes. Dark mode uses macOS Terminal.app's **Clear Dark** colors with matching
blue-gray controls. Light mode uses a soft white surface and darker ANSI colors.
Both use solid backgrounds; changing appearance keeps running sessions intact.
Neutral RGB prompt/message backgrounds near the old theme surface adapt too,
so agent TUIs do not retain dark strips after switching to light mode.

Press **Cmd-P** and choose **Customize DevDesk with Claude…** or
**Customize DevDesk with Codex…** to describe a change. For example: “Use warm-dark, make terminal text
16, and set the sidebar to 240.” The agent opens as an ordinary embedded TUI and
uses its existing CLI authentication and permissions; its proposal instructions
are not a separate security sandbox.

When the agent finishes, choose **Review Customization…** in Cmd-P. The review shows added/removed lines and the complete
proposed configuration, including process commands and autostart. **Apply changes**
validates the proposal and updates the app. Fonts, theme, sidebar width, and
Option-as-Meta update existing terminals live. Scrollback limits apply to newly
started sessions; running process definitions take effect on process restart.
Autostart follows the existing project-open behavior. **Undo Last Customization** in Cmd-P restores the last
applied configuration, including after an app restart, and refuses to overwrite
subsequent edits. It does not undo executed commands or close opened terminals.

Supported themes are `system`, `dark`, `light`, and `warm-dark`; optional
`settings.sidebar_width` accepts 220–520 pixels. The agent can also adjust project
names, default providers, process definitions, and named terminal presets.
Presets appear as **Open Terminal Preset: …** in Cmd-P; their optional `cwd` can
be absolute, home-relative, or relative to the project. Project IDs and paths
must stay intact; use the app's project controls to add/remove projects.
Generated panels and source-code changes are outside this first version.

Proposals and the last Undo snapshot live beside `config.toml` in private
`customization/` files. Each conversation has its own folder and configuration
snapshot. Review targets the latest conversation. If configuration changes while
the agent works, start a new customization; if the proposal changes after review,
review it again. Invalid proposals leave the live configuration untouched.

Status circles in the sidebar use green for running processes and idle
agents, blue for working agents, amber for transitions or attention, red for
failures, and muted outlines for stopped or exited sessions.

Navigate sessions in the project sidebar. **Processes → +** opens a modal for
the process name and command. The definition is saved to configuration and
appears stopped; use its start control to run it in the project folder.
**Terminals → +** opens a shell;
**Agents → +** offers Launch Claude and Launch Codex. Shell rows have a close
button; process and agent rows keep their start/stop/resume and remove controls.
Each session row shows its approximate process-tree memory in MB, sampled every
five seconds. This excludes terminal scrollback stored in DevDesk. A dash means
the session is stopped, has not been sampled yet, or its memory is unavailable.
Agent status and provider details are available by hovering over its name.

Configuration lives at `~/Library/Application Support/DevDesk/config.toml`.
Add projects with Cmd-O; add process definitions with **Processes → +**, or edit them in that global file (the app
watches it and keeps the last valid configuration after an error). Example:

```toml
[settings]
scrollback_lines = 5000
font_family = "Menlo"
font_size = 14

[[projects]]
id = "my-app"
name = "My App"
path = "/absolute/path/to/my-app"

[projects.agents]
default_provider = "claude" # or "codex"

[[projects.processes]]
id = "vite"
name = "Vite"
command = "npm run dev"
autostart = false

[[projects.processes]]
id = "horizon"
name = "Horizon"
command = "php artisan horizon"
autostart = false

[[projects.processes]]
id = "serve"
name = "Artisan Serve"
command = "php artisan serve"
autostart = false
```

🧪 Tested CLI versions: Claude Code **2.1.295**, Codex **0.162.0**. Version
changes produce a warning; use the documented probes before trusting changed
flag behavior. Real probes make model calls; ordinary checks do not.
DevDesk resolves the login/interactive shell environment once per app launch,
then uses `$SHELL -c` for configured commands. Restart the app after changing
shell startup files or tool-manager initialization.

| Action | Keyboard |
|---|---|
| Command palette | Cmd-P; type, Up/Down, Enter; Esc to dismiss |
| Add project / new terminal | Cmd-O / Cmd-T |
| Close focused terminal or side / rename | Cmd-W / Cmd-Shift-R |
| Next / previous session; first nine sessions | Cmd-Shift-] / Cmd-Shift-[; Cmd-1…9 |
| Read-only agent question | Cmd-Shift-A, type question, Enter |
| Sidebar width | Drag the divider; Cmd-P → Widen Sidebar / Narrow Sidebar; width is saved |
| Focus primary / side; widen / narrow side | Search those commands in Cmd-P |
| Launch/resume/stop agents; start/stop/restart processes | Cmd-P |
| Open hidden tab or side history; remove project/process/agent | Cmd-P; removal asks for confirmation |
| Reload config; recover previous sessions; quit | Cmd-P; Cmd-Q also quits |
| Select all / copy / paste / clear history | Cmd-A / Cmd-C / Cmd-V / Cmd-K |
| Scroll history by page / to top or bottom | Shift-PageUp/Down / Cmd-Home/End |
| Open a visible terminal hyperlink | Cmd-P → Open Link |

Closing a process tab hides it; stopping and removing it are separate explicit
commands. Quit confirms running work and stops owned jobs. Closing the window
hides the app; it does not leave jobs unmanaged. Claude's prompt suggestion is
accepted with Right Arrow in the tested TUI; Tab is forwarded as terminal input.

Agent status for Claude comes from hooks. Codex uses notify, public terminal
titles, input and BEL: **Active (inferred)** and **Attention** are deliberately
approximate. Side conversations always restore their restricted role when
resumed. Claude sides have no shell or write tools; Codex sides use its read-only
sandbox, disable configured MCP servers and action features, and fail closed
when configuration cannot be safely read. No transcripts or rollout files are
parsed. See [agent integration](docs/agent-integration.md) for the exact flags.

SQLite stores lifecycle identities, agent IDs and layout. Terminal output stays
in bounded memory. Diagnostics live in `~/Library/Logs/DevDesk`, with capped
current/previous logs and a separate panic log containing source locations rather
than panic payloads. Crash recovery offers verified surviving sessions for
termination; a detached Codex job born and orphaned between journal scans can
escape discovery, so recovery remains best effort.

Reproduce the final measurements with `scripts/perf/run-m7.sh`, then compare
records using `scripts/perf/compare.py perf/baseline-m2.json perf/baseline.json`.
Leave benchmark windows visible and untouched. Process memory and CLI memory
are reported separately. A11's upstream GPUI display-link callbacks still prevent
a passing visible idle CPU budget; the review records that limit explicitly.

The architecture and 27 accepted ADRs are in [docs](docs/architecture.md).
Vendored Apache/MIT modifications are attributed in [vendor/README.md](vendor/README.md).
No code was copied from Zed's GPL crates.
