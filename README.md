# Tervy

<p align="center">
<a href="https://github.com/jringeisen/tervy/actions/workflows/ci.yml"><img src="https://github.com/jringeisen/tervy/actions/workflows/ci.yml/badge.svg?branch=main" alt="Build Status"></a>
<a href="https://github.com/jringeisen/tervy/releases"><img src="https://img.shields.io/github/downloads/jringeisen/tervy/total" alt="Total Downloads"></a>
<a href="https://github.com/jringeisen/tervy/releases/latest"><img src="https://img.shields.io/github/v/release/jringeisen/tervy" alt="Latest Stable Version"></a>
<a href="https://github.com/jringeisen/tervy/blob/main/LICENSE"><img src="https://img.shields.io/github/license/jringeisen/tervy" alt="License"></a>
</p>

Tervy is a Mac workspace for your projects, terminals, development commands, and
coding agents. Keep a shell or dev server running while you switch projects, and
come back to your sessions later. Coding agents are optional.

**Requirements:** An Apple Silicon Mac running macOS 15 or later. The experimental
meeting recorder requires macOS 26 or later. Tervy is currently a public beta.

## Install Tervy

1. Open the [latest Tervy release](https://github.com/jringeisen/tervy/releases/latest)
   and follow its Apple Silicon `.dmg` download link.
2. Open the downloaded disk image and drag **Tervy** to **Applications**.
3. Open **Tervy** from Applications. You do not need to build anything or install
   a coding agent to get started.

The release page also has the version's notes and SHA-256 checksum. If you are
replacing an older Tervy build, quit it before dragging the new app into
Applications.

## Your first five minutes

1. Read the **Welcome to Tervy** screen. It shows whether your shell and any
   supported coding agents were found. Agents marked “Optional · not found” are
   fine; you can use Tervy with just a terminal.
2. Click **+** beside **Projects** and choose a folder on your Mac, or press
   **⌘O**. Tervy opens that folder as a project.
3. Click **+** beside **Terminals**, or press **⌘T**, to open a shell in the
   project. Run your usual commands there.
4. If you have a development command you use often, click **+** beside
   **Processes** to save it. You can start, stop, and restart it from the
   sidebar.
5. Press **⌘P** to search Tervy's commands. You can reopen the welcome screen
   with **Setup and Prerequisites**.

Tervy keeps your projects and session layout locally. Closing the window hides
the app; use **⌘Q** to quit. Tervy asks before stopping running sessions when
you quit or install an update.

## Add a coding agent (optional)

Tervy can launch installed **Claude Code, Codex, Gemini CLI, OpenCode, and
Aider** in your project. Install and sign in to the agent's own CLI first. Then
click **+** beside **Agents** and choose the agent. If it does not appear, use
**Setup and Recheck Agents…** in that menu; restart Tervy if you recently changed
your shell PATH.

Claude Code and Codex have session resume and read-only side questions in Tervy.
The other agents run as interactive terminals with basic start and stop controls.
See the [user guide](docs/user-guide.md) for agent details, notes, skills,
meetings, shortcuts, and backups.

## Get updates

Tervy checks for published app updates. When one is available, a badge appears
on the **Settings** gear. Open Settings to choose **Check for updates** or
**Install update**. If sessions are running, Tervy asks before stopping them for
installation. You can also find every release on the
[releases page](https://github.com/jringeisen/tervy/releases).

If you installed a build older than 0.1.7, download the latest DMG and replace
the app once to enable in-app updates. Updates are published app releases; a
source-code push alone does not install a new version on your Mac.

## Help and more

- [User guide](docs/user-guide.md) — projects, agents, meetings, settings,
  shortcuts, and local data.
- [Support](SUPPORT.md) — troubleshooting and bug reports.
- [Contributing](CONTRIBUTING.md) — build and development setup.
- [Development and release notes](docs/development.md) — technical details for
  building and publishing Tervy.
- [Release notes](https://github.com/jringeisen/tervy/releases) — downloads and
  changes in each version.

Tervy's application code is [MIT licensed](LICENSE). Original Tervy
brand assets have separate [usage terms](assets/brand/README.md#asset-rights),
and vendored dependencies retain their own licenses and attribution in
[vendor/README.md](vendor/README.md).
