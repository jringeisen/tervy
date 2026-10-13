#!/usr/bin/env python3
"""Release hook-helper wall time, including exec/dyld and real local socket I/O.
No GUI window, model call, or user configuration. Warmed and first runs separate.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import statistics
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/tervy'))
    parser.add_argument('--runs', type=int, default=100)
    parser.add_argument('--output', type=Path, default=Path('perf/local/m4-hook.json'))
    args = parser.parse_args()
    binary = str(args.binary.resolve())
    payload = b'{"hook_event_name":"PostToolUse","session_id":"10000000-0000-4000-8000-000000000001","tool_name":"Read","tool_use_id":"toolu_probe"}'
    samples = []
    with tempfile.TemporaryDirectory(prefix='dd-hook-', dir='/tmp') as folder:
        path = str(Path(folder) / 'hook.sock')
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(path)
        os.chmod(path, 0o600)
        listener.listen(16)
        received = []

        def collect():
            for _ in range(args.runs + 5):
                connection, _ = listener.accept()
                with connection:
                    connection.settimeout(1)
                    data = b''
                    while not data.endswith(b'\n'):
                        data += connection.recv(65536)
                    received.append(json.loads(data))
        thread = threading.Thread(target=collect, daemon=True)
        thread.start()
        env = dict(os.environ, TERVY_AGENT_ID='10000000-0000-4000-8000-000000000001', TERVY_LAUNCH_ID='1',
                   TERVY_HOOK_TOKEN='20000000-0000-4000-8000-000000000002', TERVY_HOOK_SOCKET=path)
        for index in range(args.runs + 5):
            start = time.perf_counter_ns()
            result = subprocess.run([binary, 'hook', 'claude'], env=env, input=payload, capture_output=True, timeout=1)
            elapsed = (time.perf_counter_ns() - start) / 1e6
            assert result.returncode == 0 and not result.stdout and not result.stderr
            if index == 0:
                first = elapsed
            if index >= 5:
                samples.append(elapsed)
        thread.join(timeout=2)
        listener.close()
        assert len(received) == args.runs + 5, 'helper did not deliver every event'
    ordered = sorted(samples)
    report = {'evidence': '🧪 release helper exec + dyld + stdin projection + real Unix socket delivery; no GPUI initialization or model call',
              'runs': args.runs, 'warmup_runs': 5, 'first_ms': first, 'p50_ms': statistics.median(samples),
              'p95_ms': ordered[round(.95*(len(ordered)-1))], 'max_ms': max(samples), 'samples_ms': samples,
              'budget_ms': 10, 'method': 'Python perf_counter around subprocess.run, including process startup and wait; hyperfine unavailable if not installed'}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps({key: value for key, value in report.items() if key != 'samples_ms'}))


if __name__ == '__main__':
    main()
