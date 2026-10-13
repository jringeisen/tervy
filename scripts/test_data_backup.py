"""Data safety checks for the offline backup and restore command."""

import io
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("data-backup.py")


def run_command(action, state, archive):
    return subprocess.run(
        [sys.executable, str(SCRIPT), action, str(archive)],
        env={**os.environ, "TERVY_HOME": str(state), "TERVY_SKILLS_HOME": str(state.parent / "skills-home")},
        capture_output=True,
        text=True,
        check=False,
    )


class DataBackupTests(unittest.TestCase):
    def test_user_skills_round_trip_without_managed_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "state"
            source.mkdir()
            (source / "config.toml").write_text("theme = 'dark'\n")
            home = root / "skills-home"
            skill = home / ".claude/skills/review-code"
            (skill / "references").mkdir(parents=True)
            (skill / "SKILL.md").write_text("---\nname: review-code\ndescription: Review code\n---\n")
            (skill / "references/checks.md").write_text("Check tests\n")
            managed = home / ".codex/skills/.system/built-in"
            managed.mkdir(parents=True)
            (managed / "SKILL.md").write_text("managed")
            archive = root / "backup.tar.gz"
            self.assertEqual(run_command("backup", source, archive).returncode, 0)
            with tarfile.open(archive, "r:gz") as bundle:
                self.assertIn("Skills/claude/review-code/SKILL.md", bundle.getnames())
                self.assertFalse(any("built-in" in name for name in bundle.getnames()))
            import shutil
            shutil.rmtree(skill)
            target = root / "restored"
            result = run_command("restore", target, archive)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((skill / "references/checks.md").read_text(), "Check tests\n")

    def test_backup_reads_previous_state_before_first_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            previous = home / "Library/Application Support/DevDesk"
            previous.mkdir(parents=True)
            (previous / "config.toml").write_text("[settings]\nfont_size = 16\n")
            archive = home / "backup.tar.gz"
            environment = {**os.environ, "HOME": str(home)}
            environment.pop("TERVY_HOME", None)
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "backup", str(archive)],
                env=environment, capture_output=True, text=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            with tarfile.open(archive, "r:gz") as source:
                self.assertEqual(source.extractfile("Tervy/config.toml").read(), b"[settings]\nfont_size = 16\n")
            self.assertFalse((home / "Library/Application Support/Tervy").exists())

    def test_round_trip_rebases_meeting_audio_into_empty_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "original"
            source.mkdir()
            (source / "config.toml").write_text("theme = 'dark'\n")
            audio = source / "meetings" / "audio.m4a"
            audio.parent.mkdir()
            audio.write_bytes(b"recorded audio")
            microphone = source / "meetings" / "microphone.m4a"
            microphone.write_bytes(b"microphone audio")
            with sqlite3.connect(source / "state.db") as database:
                database.execute("CREATE TABLE meetings (id TEXT PRIMARY KEY, record TEXT)")
                database.execute(
                    "INSERT INTO meetings VALUES (?, ?)",
                    ("meeting-1", json.dumps({
                        "audio_path": str(audio),
                        "microphone_path": str(microphone),
                    })),
                )

            archive = root / "backup.tar.gz"
            result = run_command("backup", source, archive)
            self.assertEqual(result.returncode, 0, result.stderr)

            target = root / "already-created-empty-directory"
            target.mkdir()
            result = run_command("restore", target, archive)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((target / "config.toml").read_bytes(), (source / "config.toml").read_bytes())
            self.assertEqual((target / "meetings" / "audio.m4a").read_bytes(), b"recorded audio")
            self.assertEqual((target / "meetings" / "microphone.m4a").read_bytes(), b"microphone audio")
            with sqlite3.connect(target / "state.db") as database:
                record = json.loads(database.execute("SELECT record FROM meetings").fetchone()[0])
                self.assertEqual(record["audio_path"], str(target / "meetings" / "audio.m4a"))
                self.assertEqual(record["microphone_path"], str(target / "meetings" / "microphone.m4a"))
                self.assertEqual(database.execute("PRAGMA integrity_check").fetchone(), ("ok",))

            # Archives made before resolved_state_path was added must restore too.
            legacy_archive = root / "legacy.tar.gz"
            with tarfile.open(legacy_archive, "w:gz") as output:
                metadata = json.dumps({"format": 1, "state_path": str(source.resolve())}).encode()
                header = tarfile.TarInfo("TERVY_BACKUP.json")
                header.size = len(metadata)
                output.addfile(header, io.BytesIO(metadata))
                for file in (source / "config.toml", source / "state.db", audio, microphone):
                    output.add(file, arcname="DevDesk/" + str(file.relative_to(source)))
            legacy_target = root / "legacy-restored"
            result = run_command("restore", legacy_target, legacy_archive)
            self.assertEqual(result.returncode, 0, result.stderr)
            with sqlite3.connect(legacy_target / "state.db") as database:
                record = json.loads(database.execute("SELECT record FROM meetings").fetchone()[0])
                self.assertEqual(record["audio_path"], str(legacy_target / "meetings" / "audio.m4a"))

    def test_existing_state_and_unsafe_archive_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "original"
            source.mkdir()
            (source / "config.toml").write_text("theme = 'dark'\n")
            archive = root / "backup.tar.gz"
            self.assertEqual(run_command("backup", source, archive).returncode, 0)

            target = root / "existing"
            target.mkdir()
            (target / "keep.txt").write_text("do not overwrite")
            result = run_command("restore", target, archive)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((target / "keep.txt").read_text(), "do not overwrite")

            unsafe = root / "unsafe.tar.gz"
            with tarfile.open(unsafe, "w:gz") as output:
                metadata = json.dumps({"format": 1, "state_path": str(source)}).encode()
                header = tarfile.TarInfo("TERVY_BACKUP.json")
                header.size = len(metadata)
                output.addfile(header, io.BytesIO(metadata))
                payload = b"escape"
                header = tarfile.TarInfo("Tervy/../../escaped")
                header.size = len(payload)
                output.addfile(header, io.BytesIO(payload))
            clean = root / "clean"
            result = run_command("restore", clean, unsafe)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / "escaped").exists())
            self.assertFalse(clean.exists())


if __name__ == "__main__":
    unittest.main()
