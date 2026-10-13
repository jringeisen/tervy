# Skills in the Library and Tervy MCP

**Library → Skills** reads global agent skill directories directly. Each skill is a folder
with a `SKILL.md` file. The Library shows only skills whose folders Tervy can edit,
and shows an agent or Shared source only when it has matching editable skills.
The **New skill** editor still offers every supported
source. The supported roots are `~/.claude/skills`, `~/.codex/skills`,
`~/.gemini/skills`, `~/.config/opencode/skills`, and `~/.agents/skills`.
The Shared source can be used by Codex, Gemini CLI, and OpenCode. Aider has no
native global `SKILL.md` directory in this integration.

Open a skill to edit its full `SKILL.md` text. The file's `name` frontmatter must
match its folder. Create a skill by choosing a source, entering a lowercase
hyphenated folder name, and inserting the template. Supporting text files can be
opened or created by relative path. Binary resources appear in the file list but
cannot be edited in Tervy's text editor. Use **Refresh** if a new top-level
source directory was created outside Tervy after launch.

Codex `.system`, Claude `synced`, linked, and unwritable skills are hidden from
the Library and `list_skills`. Tervy does not edit those directories.
A saved skill is available according to each provider's own skill discovery and
reload rules. Editing a skill does not inject its text into an active terminal.

## MCP tools

Tervy exposes the local stdio server `tervy_local` to launched Claude Code,
Codex, Gemini CLI, and OpenCode sessions. Its skill tools use the same files as
the Library:

| Tool | Key arguments | Result |
| --- | --- | --- |
| `list_skills` | Optional `query`, `limit`, `offset` | Editable skill source, name, description, revision |
| `read_skill` | `id` | Full `SKILL.md` and supporting file names |
| `read_skill_file` | `id`, relative `path` | Text and revision |
| `write_skill` | `source`, `name`, full `content`; `id` and `expected_revision` on update | Saved skill |
| `write_skill_file` | `id`, relative `path`, `content`; `expected_revision` on update | New revision |
| `delete_skill` | `id`, `revision` | Deleted ID after Tervy confirmation |

`source` is one of `claude`, `codex`, `shared`, `gemini`, or `opencode`.
`id` is returned by list/read tools; callers cannot provide an arbitrary filesystem
root. `SKILL.md` and supporting text files are limited to 128 KiB per file.
Stale revisions are rejected. Only primary agent sessions can write, while side
conversations can list and read. Every request must carry a live launch identity
and token. Agent-requested deletion opens a Tervy confirmation dialog.
The meeting tools (`list_meetings`, `read_meeting`, `edit_meeting`) remain available.
The Library also provides [scratchpad notes](notes-mcp.md) and note MCP tools.
Aider does not receive MCP tools through this integration.

## Transition and backups

The old prompt library and its MCP tools are removed. On the first database
upgrade, Tervy makes a one-time `state.prompts-before-skills.db` backup beside
`state.db` if old prompts exist, then removes the old prompt table. It does not
convert them to skills. Tervy's offline backup script includes user-owned global
skill folders and their supporting files, while omitting Codex `.system` and
Claude `synced` directories. Restore refuses to overwrite an existing skill.

Skills are instructions that may include scripts or tool permissions. Review a
skill and its supporting files before running it through an agent.
