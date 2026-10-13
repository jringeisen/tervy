#!/usr/bin/env python3
"""Offline backup/restore of Tervy's local state. Quit Tervy first."""

import argparse
import fcntl
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sqlite3
import tarfile
import tempfile


def state_path():
    if "TERVY_HOME" in os.environ:
        return Path(os.environ["TERVY_HOME"])
    current = Path.home() / "Library/Application Support/Tervy"
    previous = Path.home() / "Library/Application Support/DevDesk"
    return previous if not current.exists() and previous.exists() else current


def skill_roots():
    home = Path(os.environ.get("TERVY_SKILLS_HOME", str(Path.home())))
    return {
        "claude": home / ".claude/skills",
        "codex": home / ".codex/skills",
        "shared": home / ".agents/skills",
        "gemini": home / ".gemini/skills",
        "opencode": home / ".config/opencode/skills",
    }


def unlocked(directory):
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    lock_name = "devdesk.lock" if directory.name == "DevDesk" else "tervy.lock"
    lock = (directory / lock_name).open("a+b")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        lock.close()
        raise SystemExit("Tervy is running. Stop sessions and quit it before backup or restore.")
    return lock


def verify_database(directory):
    database = directory / "state.db"
    if database.exists():
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
            result = connection.execute("PRAGMA integrity_check").fetchone()
            if result != ("ok",):
                raise ValueError(f"state.db integrity check failed: {result}")


def backup(source, archive):
    if not source.exists():
        raise SystemExit(f"No Tervy state at {source}")
    if archive == source or source in archive.parents:
        raise SystemExit("Save the backup outside the Tervy state directory")
    if archive.exists():
        raise SystemExit(f"Backup already exists: {archive}. Choose a new filename.")
    with unlocked(source):
        verify_database(source)
        archive.parent.mkdir(parents=True, exist_ok=True)
        descriptor, temporary = tempfile.mkstemp(prefix=".tervy-backup-", suffix=".tar.gz", dir=archive.parent)
        os.close(descriptor)
        try:
            with tarfile.open(temporary, "w:gz") as output:
                metadata = json.dumps({
                    "format": 2,
                    "state_path": str(source.absolute()),
                    "resolved_state_path": str(source.resolve()),
                }).encode()
                header = tarfile.TarInfo("TERVY_BACKUP.json")
                header.size = len(metadata)
                header.mode = 0o600
                output.addfile(header, io.BytesIO(metadata))
                for path in sorted(source.rglob("*")):
                    relative = path.relative_to(source)
                    if relative.parts[0] in {"tervy.lock", "devdesk.lock", "run"}:
                        continue
                    if path.is_symlink() or not (path.is_file() or path.is_dir()):
                        raise ValueError(f"Unsupported state entry: {relative}")
                    output.add(path, arcname=Path("Tervy") / relative, recursive=False)
                for source_id, root in skill_roots().items():
                    if not root.exists():
                        continue
                    if root.is_symlink() or not root.is_dir():
                        raise ValueError(f"Unsupported skill source: {root}")
                    for skill in sorted(root.iterdir()):
                        if skill.name in {".system", "synced"} or skill.name.startswith("."):
                            continue
                        if skill.is_symlink() or not skill.is_dir() or not (skill / "SKILL.md").is_file():
                            continue
                        for path in [skill, *sorted(skill.rglob("*"))]:
                            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                                raise ValueError(f"Unsupported skill entry: {path}")
                            output.add(path, arcname=Path("Skills") / source_id / path.relative_to(root), recursive=False)
            os.replace(temporary, archive)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)
    print(f"Backup saved: {archive}")


def restore(target, archive):
    if target.exists() and any(target.iterdir()):
        raise SystemExit(f"Restore target is not empty: {target}. Move it aside after quitting Tervy, then retry.")
    if not archive.is_file():
        raise SystemExit(f"Backup not found: {archive}")
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".tervy-restore-", dir=target.parent))
    try:
        with tarfile.open(archive, "r:gz") as source:
            members = source.getmembers()
            metadata_member = source.getmember("TERVY_BACKUP.json")
            metadata = json.load(source.extractfile(metadata_member))
            if metadata.get("format") not in {1, 2}:
                raise ValueError("Unsupported backup format")
            original_roots = [Path(metadata["state_path"])]
            if metadata.get("resolved_state_path"):
                original_roots.append(Path(metadata["resolved_state_path"]))
            for member in members:
                if member.name == "TERVY_BACKUP.json":
                    continue
                parts = PurePosixPath(member.name).parts
                if len(parts) < 2 or parts[0] not in {"Tervy", "DevDesk", "Skills"} or any(part in {"", ".", ".."} for part in parts):
                    raise ValueError(f"Unsafe backup path: {member.name}")
                if parts[0] == "Skills" and (len(parts) < 3 or parts[1] not in skill_roots() or parts[2] in {".system", "synced"} or (member.isfile() and len(parts) < 4)):
                    raise ValueError(f"Unsafe skill backup path: {member.name}")
                if not (member.isfile() or member.isdir()):
                    raise ValueError(f"Unsupported backup entry: {member.name}")
                destination = staging.joinpath(*parts) if parts[0] == "Skills" else staging.joinpath(*parts[1:])
                if member.isdir():
                    destination.mkdir(parents=True, exist_ok=True, mode=0o700)
                else:
                    destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                    with source.extractfile(member) as input_file, destination.open("xb") as output_file:
                        shutil.copyfileobj(input_file, output_file)
                    destination.chmod(0o600)
        skills_staging = staging / "Skills"
        for source_id, root in skill_roots().items():
            staged_root = skills_staging / source_id
            if not staged_root.exists():
                continue
            if root.is_symlink():
                raise ValueError(f"Skill source is a symlink: {root}")
            for skill in staged_root.iterdir():
                if (root / skill.name).exists() or (root / skill.name).is_symlink():
                    raise ValueError(f"Skill already exists; restore will not overwrite it: {root / skill.name}")
        relocate_meeting_paths(staging, original_roots, target)
        verify_database(staging)
        if skills_staging.exists():
            for source_id, root in skill_roots().items():
                staged_root = skills_staging / source_id
                if staged_root.exists():
                    root.mkdir(parents=True, exist_ok=True, mode=0o700)
                    for skill in staged_root.iterdir():
                        os.replace(skill, root / skill.name)
            shutil.rmtree(skills_staging)
        if target.exists():
            target.rmdir()
        os.replace(staging, target)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    print(f"State restored: {target}")


def relocate_meeting_paths(staging, original_roots, target):
    database = staging / "state.db"
    if not database.exists() or target in original_roots:
        return
    with sqlite3.connect(database) as connection:
        has_meetings = connection.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='meetings'"
        ).fetchone()
        if not has_meetings:
            return
        for meeting_id, raw in connection.execute("SELECT id, record FROM meetings").fetchall():
            meeting = json.loads(raw)
            changed = False
            for key in ("audio_path", "microphone_path"):
                path = meeting.get(key)
                if not path:
                    continue
                audio_path = Path(path)
                for original_root in original_roots:
                    if audio_path.is_relative_to(original_root):
                        meeting[key] = str(target / audio_path.relative_to(original_root))
                        changed = True
                        break
                else:
                    # Older backups stored only the resolved root. The path in
                    # a record may still use /var instead of /private/var.
                    resolved = audio_path.resolve(strict=False)
                    for original_root in original_roots:
                        if resolved.is_relative_to(original_root):
                            meeting[key] = str(target / resolved.relative_to(original_root))
                            changed = True
                            break
            if changed:
                connection.execute(
                    "UPDATE meetings SET record=? WHERE id=?",
                    (json.dumps(meeting), meeting_id),
                )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["backup", "restore"])
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    path = state_path()
    if args.action == "backup":
        backup(path, args.archive.expanduser().resolve())
    else:
        # Existing state must be moved manually so restoring never overwrites it.
        if path.exists() and any(path.iterdir()):
            with unlocked(path):
                pass
        restore(path, args.archive.expanduser().resolve())


if __name__ == "__main__":
    main()
