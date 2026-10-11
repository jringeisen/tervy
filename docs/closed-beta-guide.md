# Closed beta guide

Tervy v1 is for solo developers on Apple Silicon Macs. The core supports
macOS 15 or later. Meeting recording is experimental and requires macOS 26
or later. This guide collects evidence for the [release checklist](v1-release-checklist.md).

## Before the session

- Give each tester the private, versioned build and its SHA-256 checksum.
  Explain that the current local review DMG is ad hoc signed; use a Developer
  ID signed and notarized candidate for the distribution acceptance test.
- Ask testers to use a project they can safely work on and to keep their own
  backup. Do not request project source, prompts, transcripts, or recordings
  in feedback. Screenshots and logs should be reviewed for private data.
- Record macOS version, chip, Tervy version, and which supported agent CLIs
  are installed. Ask whether the tester has used Tervy before.

## First session (observe without coaching)

1. Install and open Tervy, add a project, and run a shell command.
2. Add a project process, edit its command, stop and restart it.
3. Start an available coding agent, stop it, quit Tervy, and return to the
   project. If no agent is installed, observe whether setup guidance helps.
4. Ask the tester to describe what they expect to persist after quitting.
5. On macOS 26, ask only willing testers to try a meeting recording. Confirm
   they understand the audio permissions and transcript delivery choice.

Record where the tester hesitates, needs help, or expects a different result.
Measure whether they complete the core loop unaided. File a separate issue
for every reproducible failure with steps, expected result, actual result,
version, and impact. Redact all private content before attaching diagnostics.

## One week of normal use

Ask testers to return to their work on at least two later days. Capture:

- Time to reopen a project and find the right session or process.
- Agent launch, sign-in, resume, and stop failures; terminal input or output
  problems; crashes or work lost after quitting.
- Whether prompts, side conversations, and process editing help their normal
  work, and which controls they cannot find.
- Whether terminal history search or basic preferences are needed to finish
  a common task, with the task and workaround recorded.
- Meeting permission, transcription, and recovery problems, separately from
  core workspace feedback.

At the end, ask: “Would you keep using Tervy for your own work? What would
make you stop?” Record the answer in the tester's words, without private
project details. Triage every issue as P0 (data loss, unsafe process control,
crash blocking core work), P1 (common task blocked or confusing), or P2
(optional polish). The public v1 gate requires no unresolved P0 and a repeatable
first-use and next-day core loop for most testers.
