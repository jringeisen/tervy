# Library and Tervy MCP

Prompts live in the **Library → Prompts** sidebar, shared across every project.
Create one in the editor, or ask an agent to draft it and then tell it to save
the result. Search matches titles and prompt text. Open a prompt to edit or delete it. **Use
with active agent** inserts its body into the selected running agent's terminal
without submitting it, so it can be reviewed first.

## Tools

Tervy exposes a local stdio MCP server named `tervy_local` to launched agents.
The tools operate on the same library the UI shows:

| Tool | Arguments | Result |
| --- | --- | --- |
| `list_prompts` | Optional `query` (title or text search), `limit` (1–100), `offset` | IDs, titles, revisions and update times |
| `read_prompt` | `id` | Full prompt including body and revision |
| `write_prompt` | `title`, `body`; for updates, `id` and `expected_revision` | Saved prompt and new revision |
| `delete_prompt` | `id`, `revision` | Deleted ID after user confirmation |
| `list_meetings` | Optional `query` (title or notes search), `limit` (1–100), `offset` | Meeting IDs, titles, status, dates, durations and revisions |
| `read_meeting` | `id` | Markdown notes, title, status and revision; no transcript or audio |
| `edit_meeting` | `id`, `expected_revision`, optional `title` and/or `notes` | Updated meeting and new revision |

For example, the agent can call `write_prompt` with
`{"title":"Minimal product design","body":"Design a polished, minimal product interface…"}`.
An update requires the current revision from `read_prompt`. A stale revision is
rejected so one editor or agent cannot silently overwrite another's change.
Titles are one line and at most 160 bytes; bodies are at most 64 KiB.

Meeting edits require the current revision from `read_meeting`; a stale revision
is rejected. `edit_meeting` changes only the title and Markdown notes. Notes
can be cleared with an empty string, but an edit must provide at least one field.
MCP reads and edits are limited to 128 KiB of notes; larger notes remain
available in the Library UI. Notes can be edited after processing finishes.
The tools do not expose transcripts, audio paths, recording controls, or meeting
deletion.

## Connections and permissions

Tervy injects the MCP server into each launched Claude Code and Codex session.
Gemini CLI receives an additive private system-defaults file. OpenCode receives
an inline config overlay. Existing provider configuration is left on disk; a
preexisting `tervy_local` server name is rejected where Tervy can detect it.
Gemini's higher-priority user, project, or system settings can still override or
exclude the server. Aider does not receive MCP tools because this integration
has no native Aider MCP connection. Its terminal can still receive a saved
prompt through the UI.

The MCP subprocess talks to Tervy's supervisor over a private local Unix
socket. The supervisor alone reads and writes `state.db`. Each request carries
the agent ID, current launch ID, and a token passed only to that launch. A
stopped, replaced, or removed agent cannot keep using its old connection.
Primary agents can read and edit Library content. Side conversations can only
list and read, even if they invoke a write operation directly. An agent's prompt
delete request opens a confirmation dialog in Tervy; cancellation or timeout leaves the prompt
intact. UI deletion also asks for confirmation.

Prompts and meeting notes are available to any authorized agent launched in
Tervy, including agents working in other projects. Keep sensitive content out
of either library if those agents should not see it.

## Verification

The core tests cover database migration, revision conflicts, the MCP socket
round trip, bounded messages, live-launch authorization, side-role restrictions,
and provider launch configuration. A local CLI probe verified Codex recognizes
the injected server. End-to-end model tool use still depends on each installed
provider version and its own tool permission settings; use its MCP status view
to check the connection after upgrading a CLI.
