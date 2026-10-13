# Notes in the Library and Tervy MCP

**Library → Notes** is a scratchpad shared across projects. Each note has a title
and optional text. Notes are stored in Tervy's `state.db` and included in offline
backups. The old prompt library is not imported into Notes.

| MCP tool | Arguments | Result |
| --- | --- | --- |
| `list_notes` | Optional `query`, `limit` (1–100), `offset` | IDs, titles, revisions, update times; no note text |
| `read_note` | `id` | Full title, text, revision, update time |
| `write_note` | `title`, `body`; for updates add `id` and `expected_revision` | Saved note and new revision |
| `delete_note` | `id`, current `revision` | Deleted ID after Tervy confirmation |

Search matches titles and note text, with newest notes first. Titles are limited
to one line and 160 bytes; note text is limited to 64 KiB. Updates reject stale
revisions so an agent cannot silently overwrite a newer edit. You can copy a
note's text from the Library editor.

Primary Claude Code, Codex, Gemini CLI, and OpenCode sessions launched in Tervy
can write notes. Read-only side conversations can list and read only. Aider does
not receive Tervy's MCP tools. Every request is checked against its live agent
session and launch token. Agent-requested deletion opens a Tervy confirmation
dialog.
