# Tervy

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
# ./scripts/bundle.sh target/Tervy-review.app
```

The last command creates `target/Tervy.app` with an original icon and
`Info.plist`, without distribution signing or notarisation. Move the app to its
final location **before launching agents**: each CLI receives the absolute path
to that bundle's `Contents/MacOS/devdesk` hook helper. Stop agents before moving
or replacing the bundle, then resume them from the new app.

### Meeting recordings

Use **Record** in the toolbar to capture both your microphone and meeting audio
playing on your Mac. **Stop** saves the audio, transcribes it on device, then
uses your signed-in Codex or Claude Code CLI to write meeting notes. Recordings,
transcripts, and editable notes appear in **Library → Meetings**. Audio stays on
your Mac by default; **Delete audio** removes both tracks while keeping the
transcript and notes. **Delete meeting** removes the whole entry. If processing
fails, the meeting stays in the Library and can be retried while its audio is
present. macOS may ask for microphone and system audio recording permission.
On-device transcription requires macOS 26 or later and a supported language.
The transcript is sent to the configured AI provider through its agent CLI to
generate notes. Saved audio is uncompressed and can use roughly 700 MB per hour
for two stereo tracks.

The whole workspace follows macOS light/dark appearance, including open terminal
panes. Dark mode uses macOS Terminal.app's **Clear Dark** terminal colors with neutral
workspace controls and a blue accent. Light mode uses soft white content surfaces,
subtle gray navigation, and darker ANSI colors. Shared controls have visible
keyboard focus; forms and customization reviews fit narrower windows.
Both use solid backgrounds; changing appearance keeps running sessions intact.
Neutral RGB prompt/message backgrounds near the old theme surface adapt too,
so agent TUIs do not retain dark strips after switching to light mode.

Press **Cmd-P** and choose **Customize Tervy with Claude…** or
**Customize Tervy with Codex…** to describe a change. For example: “Use warm-dark, make terminal text
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
**Agents → +** discovers installed Claude Code, Codex, Gemini CLI, OpenCode, and
Aider executables on your login-shell PATH and offers only those agents.
Use **Setup and Recheck Agents…** after installing a CLI. Gemini CLI, OpenCode,
and Aider support interactive launch, stop, and restart with a Running status;
conversation resume and read-only side questions are integrated for Claude and
Codex. Shell rows have a close
button; process and agent rows keep their start/stop/resume and remove controls.

The **Prompts** section in the Library sidebar stores reusable prompts across all projects.
Search, create, edit, copy, or delete them there. **Use with active agent** places
the saved text in the selected running agent's terminal for review before sending.
Claude Code, Codex, Gemini CLI, and OpenCode launched in Tervy receive the local
`tervy_local` MCP tools for prompts and meeting notes. These include
`list_meetings`, `read_meeting`, and `edit_meeting`. An agent can draft a prompt
or update meeting notes when asked. Agent prompt deletions require confirmation
in Tervy; read-only side conversations get only the list and read tools. Aider
has no native MCP connection in this integration;
use the Library's copy or **Use with active agent** action for Aider. The prompt
and meeting records are stored in Tervy's local SQLite state database. See
[Library and MCP](docs/prompts-mcp.md) for tool arguments and provider details.
Before launching or resuming a primary agent, edit its arguments in the launch
dialog. For example, enter `--approve-for-me` for Codex. Quoted values with spaces
are supported. Arguments are saved with each session, and new sessions prefill
the last saved arguments for that agent type. Read-only side conversations keep
their managed launch settings.
Use the toolbar sidebar button, **Cmd-B**, or **Hide/Show Sidebar** in Cmd-P to
collapse or reopen the sidebar. The collapsed state and width are remembered;
**Cmd-0** reopens it and focuses its keyboard navigation.
At sidebar widths of 280 pixels and above, each session row shows its approximate
process-tree memory in MB, sampled every five seconds. Narrower sidebars prioritize
session names and controls; memory remains in the session tooltip. This excludes
terminal scrollback stored in Tervy. A dash means
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
Tervy resolves the login/interactive shell environment once per app launch,
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
sandbox, disable other configured MCP servers and action features, and fail closed
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

## Brand and Mac beta

The public brand is **Tervy**: “Keep your tools together. Keep your agents moving.”
Its modular **T** brings a tool tile and shared bar together with a forward arrow.
The palette uses cobalt, graphite, paper, and pale blue supporting graphics.
System sans-serif is used for interface copy, with Menlo for technical text.
Copy should be clear, capable, and calm.

The [brand sheet](assets/brand/brand-sheet.svg) shows the logo, tagline, palette,
and applications. The [brand guide](assets/brand/README.md) documents usage and
exports. The [vector kit](assets/brand) includes light/dark marks, outlined
wordmarks, tagline lockups, and the app icon. `identity.json` is the source for
geometry and colors; run `python3 scripts/brand.py` to regenerate the SVG assets.
The native icon renderer reads the same source.

Use **+** beside **Projects** (or **Cmd-O**) to add a local folder. The sidebar
keeps project and session controls together; **Cmd-P** opens all commands,
including **Setup and Prerequisites**. Click the sidebar or press **Cmd-0** to
navigate visible project and session rows with **Up/Down**. **Enter** opens the
highlighted item and focuses its terminal; **Escape** returns to the current
terminal. **Left/Right** collapse or expand projects. Navigation skips collapsed
sections and does not launch sessions until you activate an item. Arrow keys
inside terminals, inputs, and command search retain their normal behavior.

A welcome dialog appears on the first launch and explains projects, terminals,
processes, agents, and shortcuts. Dismissing it remembers that it has been seen;
you can reopen it through **Setup and Prerequisites**. The welcome dialog and
empty workspace check the login-shell environment for a usable shell and
optional Claude Code, Codex, Gemini CLI, OpenCode, and Aider executables. These checks do not
run agents, install software, or verify account authentication. Install and sign
in to agent CLIs separately, then restart Tervy to refresh its shell environment.

Build a local review disk image on an Apple Silicon Mac:

```sh
scripts/release.sh --local
```

The image in `target/releases` includes Tervy, an Applications shortcut, and
installation instructions. A SHA-256 checksum and machine-readable release
manifest accompany it. Local images are **unsigned and not notarized**; they are
review artifacts, not public releases. A manual GitHub Actions workflow builds
these artifacts without publishing a release.

For a public Mac release, configure a Developer ID Application certificate and a
`notarytool` Keychain profile using Apple's
[distribution guidance](https://developer.apple.com/developer-id/), then run:

```sh
export DEVDESK_SIGNING_IDENTITY='Developer ID Application: Your Name (TEAMID)'
export DEVDESK_NOTARY_PROFILE='devdesk-notary'
scripts/release.sh
```

The script checks the workspace, builds the app, signs with the hardened runtime,
notarizes and staples the app, creates and signs a disk image, then notarizes,
staples, and verifies the image. It fails if notarization is not accepted. No
credentials belong in source control. The signing branch must be tested with real
credentials before publishing. Updates currently require replacing the installed
app after stopping sessions and quitting; an automatic updater is not included.

This beta targets **macOS on Apple Silicon**. Intel Mac, Windows, and Linux builds
are not yet validated. Cross-platform work remains in terminal/process ownership,
IPC, shell discovery, app integration, and packaging. Downloads do not sync local
settings or project files. Before a public launch, resolve domain availability
and name availability for Tervy; this brand kit
does not establish exclusive rights to the name.
