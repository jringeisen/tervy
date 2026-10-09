#!/usr/bin/env python3
"""Capture only terminal startup probes, never agent transcripts or rollouts.

Run from the DevDesk repository with an installed agent: --agent claude|codex.
No prompt is submitted and no model turn is requested. Human-visible output
stays in memory and is discarded; committed fixtures contain queries/replies
and version/launch metadata only. Review fixture changes after CLI upgrades.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import termios
import time

# A fresh 120x40 terminal at 1;1 with the deterministic dark test palette.
REPLIES = {
    b"\x1b[6n": b"\x1b[1;1R", b"\x1b[?6n": b"\x1b[?1;1R",
    b"\x1b[c": b"\x1b[?6c", b"\x1b[0c": b"\x1b[?6c",
    b"\x1b[>c": b"\x1b[>0;1;0c", b"\x1b[>0c": b"\x1b[>0;1;0c",
    b"\x1b[?u": b"\x1b[?0u", b"\x1b[18t": b"\x1b[8;40;120t",
    b"\x1b[14t": b"\x1b[4;720;960t",
}
SEQUENCE = re.compile(rb"\x1b\[[0-9?;>=:]*[a-zA-Z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")

def reply_for(query):
    if query in REPLIES:
        return REPLIES[query]
    for index, rgb in [(10, "d8d8/dede/e9e9"), (11, "2020/2424/2c2c"), (12, "d8d8/dede/e9e9")]:
        if query in [f"\x1b]{index};?\x07".encode(), f"\x1b]{index};?\x1b\\".encode()]:
            end = b"\x07" if query.endswith(b"\x07") else b"\x1b\\"
            return f"\x1b]{index};rgb:{rgb}".encode() + end
    return None

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", choices=["claude", "codex"], required=True)
    parser.add_argument("--seconds", type=float, default=8)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raw_env = subprocess.check_output(["/bin/zsh", "-l", "-i", "-c", "printf '\\0DEVDESK_ENV\\0'; env -0"], stderr=subprocess.DEVNULL, timeout=5)
    env = dict(os.environ)
    for entry in raw_env.split(b"\0DEVDESK_ENV\0", 1)[1].split(b"\0"):
        if b"=" in entry:
            key, value = entry.split(b"=", 1)
            env[os.fsdecode(key)] = os.fsdecode(value)
    env.update(TERM="xterm-256color", COLORTERM="truecolor", TERM_PROGRAM="DevDesk", CLAUDE_CODE_NONBLOCKING_STDOUT="1")
    argv = [args.agent] + (["--no-daemon"] if args.agent == "codex" else [])
    version = subprocess.check_output([args.agent, "--version"], env=env, text=True, timeout=5).strip()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 960, 720))
    start = time.monotonic()
    child = subprocess.Popen(argv, env=env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True, preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0))
    os.close(slave)
    exchanges = []
    injected_at_ms = None
    pending = b""
    first_output_ms = None
    try:
        while time.monotonic() - start < args.seconds and child.poll() is None:
            if injected_at_ms is None and time.monotonic() - start >= 4:
                os.write(master, b"\x1b[13;2u")
                injected_at_ms = round(1000 * (time.monotonic() - start), 3)
            ready, _, _ = select.select([master], [], [], .05)
            if not ready:
                continue
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            if first_output_ms is None:
                first_output_ms = 1000 * (time.monotonic() - start)
            pending += chunk
            end = 0
            for match in SEQUENCE.finditer(pending):
                end = match.end()
                query = match.group()
                reply = reply_for(query)
                if reply is not None:
                    before = time.monotonic()
                    os.write(master, reply)
                    exchanges.append({"query_hex": query.hex(), "reply_hex": reply.hex(), "at_ms": round(1000 * (before - start), 3)})
            # Keep only a possible incomplete escape sequence, with a bound.
            pending = pending[end:][-8192:]
    finally:
        os.close(master)
        os.killpg(child.pid, signal.SIGTERM) if child.poll() is None else None
        try:
            child.wait(timeout=2)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=2)
    result = {"agent": args.agent, "version": version, "argv": argv, "date": "2026-10-08",
              "dimensions": {"columns": 120, "rows": 40, "cell_width": 8, "cell_height": 18},
              "method": "real CLI startup in PTY, no submitted prompt; protocol probes only",
              "first_pty_output_ms": first_output_ms, "exchanges": exchanges,
              "keys": [{"key": "Enter", "shift": True, "injected_at_ms": injected_at_ms, "legacy_hex": "1b5b31333b3275", "kitty_flags_1_hex": "1b5b31333b3275"}]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"version": version, "probe_count": len(exchanges), "first_pty_output_ms": first_output_ms}))

if __name__ == "__main__":
    main()
