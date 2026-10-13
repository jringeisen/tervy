# Using Tervy

Start with the [installation and first-project steps](../README.md) if you are new to Tervy.

## First launch

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

## Sessions and projects

Status circles in the sidebar use green for running processes and idle
agents, blue for working agents, amber for transitions or attention, red for
failures, and muted outlines for stopped or exited sessions.

Navigate sessions in the project sidebar. **Processes → +** opens a modal for
the process name and command. The definition is saved to configuration and
appears stopped; use its start control to run it in the project folder.
Use **Edit** on the process row to change its name, command, or autostart. A
running process uses the old command until you restart it.
**Terminals → +** opens a shell;
**Agents → +** discovers installed Claude Code, Codex, Gemini CLI, OpenCode, and
Aider executables on your login-shell PATH and offers only those agents.
Use **Setup and Recheck Agents…** after installing a CLI. Gemini CLI, OpenCode,
and Aider support interactive launch, stop, and restart with a Running status;
conversation resume and read-only side questions are integrated for Claude and
Codex. Shell rows have a close
button; process and agent rows keep their start/stop/resume and remove controls.

## Skills and notes

The **Skills** section in the Library sidebar reads global agent skill folders.
Expand any agent shown under Skills to browse its installed skills, then
open `SKILL.md` to view or edit it. You can create skills and edit supporting
text files there too. Bundled, synced, linked, and otherwise unwritable skills
are hidden from the Library. Claude Code, Codex, Gemini CLI, and OpenCode launched in Tervy
receive `tervy_local` MCP tools for skills and meeting notes. Primary agents
can create and edit user-owned skills; read-only side conversations can only
list and read them. Agent skill deletions require confirmation in Tervy.
Aider has no native skill directory or MCP connection in this integration.
See [Skills and MCP](skills-mcp.md) for paths, tool arguments, and provider
details.
The **Notes** section is a shared scratchpad in the Library. Create notes with a
title and optional text, search them, and edit or delete them across projects.
Use **Cmd-N** from any screen to open a new note dialog. Saved notes have a
**Delete** button in the editor and a trash button in search results; deletion
asks for confirmation.
Primary agents can list, read, create, and edit notes through `tervy_local` MCP;
read-only side conversations can list and read. Agent-requested deletion needs
confirmation in Tervy. Notes are stored in `state.db` and included in offline
backups. See [Notes and MCP](notes-mcp.md) for tool arguments.
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

## Experimental meeting recordings

This feature is experimental in v1 and requires more signed-build and privacy
validation before it is considered a dependable recording tool. Use **Record**
in the toolbar to capture both your microphone and meeting audio
playing on your Mac. Choose to skip AI notes or send the transcript to your
signed-in Codex or Claude Code CLI for notes. **Stop** saves the audio and
transcribes it on device. Recordings,
transcripts, and editable notes appear in **Library → Meetings**. Audio stays on
your Mac by default; **Delete audio** removes both tracks while keeping the
transcript and notes. **Delete meeting** removes the whole entry. If processing
fails, the meeting stays in the Library and can be retried while its audio is
present. macOS may ask for microphone and system audio recording permission.
If capture reports a permission error, open System Settings → Privacy & Security →
Screen & System Audio Recording and allow Tervy Meeting Audio (or Tervy) to
record system audio. Also check Microphone access there, then quit and reopen
Tervy before retrying.
On-device transcription requires macOS 26 or later and a supported language.
If selected, the transcript is sent to that AI provider through its agent CLI to
generate notes. Agents you launch with Tervy's meeting tools can read Library
meetings. Saved audio is uncompressed and can use roughly 700 MB per hour
for two stereo tracks.

## Appearance

The whole workspace follows macOS light/dark appearance, including open terminal
panes. Dark mode uses macOS Terminal.app's **Clear Dark** terminal colors with neutral
workspace controls and a blue accent. Light mode uses soft white content surfaces,
subtle gray navigation, and darker ANSI colors. Shared controls have visible
keyboard focus; forms and customization reviews fit narrower windows.
Both use solid backgrounds; changing appearance keeps running sessions intact.
Neutral RGB prompt/message backgrounds near the old theme surface adapt too,
so agent TUIs do not retain dark strips after switching to light mode.

## Customize Tervy

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

## Keyboard shortcuts

| Action | Keyboard |
|---|---|
| Command palette | Cmd-P; type, Up/Down, Enter; Esc to dismiss |
| Add project / new terminal | Cmd-O / Cmd-T |
| New note from any screen | Cmd-N |
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
Drag files or folders from Finder onto a terminal or agent pane to insert their
paths at the cursor. Multiple paths are separated by spaces; Tervy quotes paths
when needed and does not submit the line.

## Configuration

Configuration lives at `~/Library/Application Support/Tervy/config.toml`.
Add projects with Cmd-O; add process definitions with **Processes → +**, or edit advanced settings in that global file (the app
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

## Agent behavior and diagnostics

Agent status for Claude comes from hooks. Codex uses notify, public terminal
titles, input and BEL: **Active (inferred)** and **Attention** are deliberately
approximate. Side conversations always restore their restricted role when
resumed. Claude sides have no shell or write tools; Codex sides use its read-only
sandbox, disable other configured MCP servers and action features, and fail closed
when configuration cannot be safely read. No transcripts or rollout files are
parsed. See [agent integration](agent-integration.md) for the exact flags.

SQLite stores lifecycle identities, agent IDs and layout. Terminal output stays
in bounded memory. Diagnostics live in `~/Library/Logs/Tervy`, with capped
current/previous logs and a separate panic log containing source locations rather
than panic payloads. Crash recovery offers verified surviving sessions for
termination; a detached Codex job born and orphaned between journal scans can
escape discovery, so recovery remains best effort.

## Back up and restore local data

Backup and restore currently use a script from the source repository. From a
local checkout of Tervy, stop sessions and quit the app, then run:

```sh
python3 scripts/data-backup.py backup ~/Desktop/tervy-backup.tar.gz
```

The archive includes configuration, user-owned global skills and supporting
files, meeting records and audio, and agent/session metadata. It does not include
project files or terminal scrollback. Keep the archive private: it can contain
personal skills and recordings. To restore, quit Tervy, move the existing
`~/Library/Application Support/Tervy` directory aside, then run:

```sh
python3 scripts/data-backup.py restore ~/Desktop/tervy-backup.tar.gz
```

The restore refuses to overwrite existing state or skills and checks the SQLite database.
On first launch, Tervy moves an existing pre-rename application data directory
to `~/Library/Application Support/Tervy` and updates saved meeting audio paths.
Quit the previous build before opening Tervy. The new bundle identifier means
macOS may ask for microphone and system audio permission again.
