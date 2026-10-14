# Tervy v1 product review and release plan

Status: working plan for an open source v1. Product decisions confirmed by the
maintainer: the primary user is a solo developer; Tervy's code uses MIT;
the official brand assets are reserved; meeting recording is experimental in v1.
The core supports Apple Silicon macOS 15+; meetings require macOS 26+.
This review is based on the current implementation, documentation, automated
checks, and a local review disk image. It is not a substitute for observed use
by new users or a test of a Developer ID signed build.

## Product promise

An Apple Silicon Mac developer can open a local project, run its shell and
project commands, work with a coding agent, stop safely, and return to the work
the next day. Claude Code and Codex are the fully integrated agent paths.
Gemini CLI, OpenCode, and Aider are interactive terminal options with fewer
session features. Tervy does not require a hosted account of its own.

The core is cohesive: projects provide context; terminals and managed processes
provide the work environment; embedded agents act inside it; read-only side
conversations let the developer ask about an active agent without changing
files; saved prompts support repeat work. The app should teach this loop before
presenting advanced configuration or the experimental meeting workflow.

## Feature review

| Area | What works | V1 improvement |
| --- | --- | --- |
| Projects and navigation | Projects, session sidebar, command palette, tab history, and keyboard navigation share one model. | Test a first project from a clean install. Make returning to yesterday's project and finding a hidden session obvious. |
| Terminal | Native PTYs, bounded scrollback, copy/paste, links, appearance, and lifecycle tests give Tervy a credible foundation. | Add a discoverable search of visible terminal history if beta users need it; explain that terminal output is not saved across launches. |
| Project processes | Add, edit name/command/autostart, start, stop, restart, and recovery are integrated. | Verify the new editor with real running processes and a restart; explain that changes to a running command apply on restart. Advanced fields remain in TOML or agent customization. |
| Claude and Codex | Launch arguments, resume, status, lifecycle ownership, and side conversations are differentiated features. | Capability labels now appear before launch. Validate sign-in or make authentication failures easy to recover from; test supported CLI versions on fresh machines and keep inferred status labeled as such. |
| Other agent CLIs | Three more installed CLIs can launch without requiring a new provider-specific UI. | Basic terminal labels now appear in the menu and launch dialog; check that the distinction is clear to new users. |
| Read-only side conversations | Contextual, constrained questions are a clear reason to use Tervy instead of separate terminal windows. | Explain the snapshot/read-only behavior in plain language at first use and make the parent/side relationship unmistakable. |
| Prompts Library | Search, edit, copy, and insertion into an active agent support the core loop. | Tell users that prompts are shared across projects and available to authorized launched agents. Test the new offline backup/restore procedure against real data before inviting users to accumulate a library. |
| Agent customization | Reviewed proposals and undo make powerful setup changes possible. | Keep it as an advanced path. Users should be able to complete ordinary setup and process edits without an agent. Continue to show command and autostart changes prominently in reviews. |
| Meetings | Recording, local transcription, retry, notes, and deletion form a substantial adjacent workflow. | The UI now labels it experimental, gates recording to macOS 26+, and offers local-only, Codex, or Claude notes delivery before capture. Before promotion, validate signed permissions and test interrupted recordings. |
| Appearance and setup | Native light/dark themes, focus states, welcome screen, and executable discovery are present. | Use a short first-success path: add project, open terminal, optionally connect an agent. State minimum supported macOS version and specific CLI capabilities. |

## Release gates, in priority order

### P0: required for a public v1

1. **Open source terms and participation.** Publish a root code license,
   explicit rights for reserved artwork, contributor instructions, a private
   security reporting path, support guidance, and pull-request CI. Preserve
   third-party notices and license files.
2. **A real distributable.** Validate the macOS 15+ minimum and Tervy app
   identity, then build, sign, notarize, install, and update a candidate on a
   clean Mac account. Confirm that earlier local state migrates to Tervy and
   recording permissions work with the new bundle identifier. A local ad-hoc
   DMG is only a review artifact.
3. **User data safety.** Validate the new offline backup and restore procedure for
   configuration, prompts, meeting text/audio, and state. Test it after a
   forced quit and across an update. Do not rely on copying a live SQLite WAL
   database without a consistent snapshot.
4. **Core workflow completion.** Verify configured process editing in the UI and
   their lifecycle across restart. A new user should be able to launch one
   supported agent, stop it, quit, and resume safely without editing TOML.
5. **Experimental recording boundaries.** Keep its experimental label in the UI and
   docs; explain that it captures microphone and Mac audio, transcription runs
   locally, and notes generation sends the transcript to the chosen provider.
   Validate this choice, permissions, and failure recovery in a signed build.
   Validate permissions in the signed app. Recording failure must preserve any
   usable audio and show a recovery action.

### P1: polish during closed beta

- Improve first-run guidance and error recovery based on observed users, not
  only synthetic UI probes. Test no CLI, one supported CLI, and a CLI on an
  unsupported version.
- Observe whether the new provider capability labels prevent mistaken expectations;
  improve authentication failure guidance where needed.
- Decide from beta evidence whether terminal history search and a basic
  preferences screen are needed for v1. Both are likely useful; neither should
  displace data safety and process editing.
- Publish a clear manual update path, release notes, and a support route. An
  automatic updater is not required for the first open source release.

### P2: after the v1 core is trusted

Cloud sync, team spaces, provider parity across all CLIs, meeting automation,
and a plugin marketplace are outside the solo-developer v1 promise. Continue
to monitor visible idle CPU and full-history memory from the M7 review; fix
user-visible battery or responsiveness problems seen in beta.

## Phases and exit criteria

| Phase | Work | Exit criterion |
| --- | --- | --- |
| 0. Scope | Confirm solo developer, core promise, supported OS and provider tiers; classify meetings experimental. | User-facing copy and issue priorities match these choices. |
| 1. Open source foundation | License, brand terms, contribution/security/support docs, PR CI, release checklist. | A stranger can clone, build, test, and report a problem without private instructions. |
| 2. Core completeness | Process editing, data backup/restore, migration, first-run and capability clarity, recording boundaries. | Fresh-install and upgrade scripts pass on supported Macs; no known data-loss or unmanaged-process blocker. |
| 3. Closed beta | Observe several solo developers on real projects for at least a week; triage friction and crashes. | Most can complete first setup unaided, return to work the next day, and recover from a failed agent or recording. No unresolved P0 issue. |
| 4. Public release | Signed/notarized artifact, checksum, versioned GitHub Release, release notes, support and rollback instructions. | A downloaded v1 installs, launches, updates, and passes the release acceptance matrix. |

## Acceptance matrix to build and record

- Fresh install: supported OS, permissions denied/granted, no CLI, Claude only,
  Codex only, several CLIs; light/dark and narrow window.
- Daily work: add and rename project, terminal input/scrollback, create/edit
  process, stop/restart, launch/resume agent, ask read-only question, quit/reopen.
- Data: import or restore a backup with prompts and meetings; upgrade an older
  Tervy state; force quit while a process or recording is active.
- Distribution: local build, signed/notarized DMG, clean account install,
  Gatekeeper check, update and rollback, artifact checksum and version display.
- Community: README build path, successful PR CI, issue/support guidance,
  license and attribution, security reporting without exposing private data.

The repository's `scripts/check.sh` currently passes formatting, Clippy,
ordinary tests, and dependency license checks. Those checks establish code
health, not first-use quality or signed-build behavior. On 2026-10-11,
[CI](https://github.com/jringeisen/tervy/actions/runs/38153364562) passed on
macOS 26 and the [manual beta workflow](https://github.com/jringeisen/tervy/actions/runs/38153630235)
built, verified, and uploaded a private review DMG. The M7 performance and crash
limits remain recorded in [m7-review.md](m7-review.md).

The first implementation pass added Apache-2.0 (now MIT) and brand terms, participation
docs and PR CI, process editing, an offline backup/restore tool, OS and meeting
boundaries, transcript delivery choice, and release checks. The local review
DMG builds and verifies on macOS 27. These are implementation checks; the
macOS 15 clean install, signed macOS 26 recording, Tervy upgrade, and beta
user observations are still open acceptance gates.
