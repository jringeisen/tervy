#!/usr/bin/env python3
"""Bounded M2 three-project/configurable noisy-session probe, with crash-safe session cleanup.
Build release app and tervy-term's session_cleanup example first.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
from idle import usage, child_usage, threads, window_state

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--noisy', type=int, choices=[10,20], default=20)
    parser.add_argument('--sample', action='store_true')
    parser.add_argument('--output', type=Path, default=Path('perf/local/m2-many.json'))
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    log_path = args.output.with_suffix('.stderr.txt')
    env = dict(os.environ, TERVY_PERF_LAYOUT='many', TERVY_PERF_SCENARIO='many', RUST_BACKTRACE='1', TERVY_PERF_NOISY=str(args.noisy))
    env.pop('TERVY_PERF_EXIT', None)
    env.pop('TERVY_PERF_T0', None)
    samples = []
    cleaned = []
    failure = None
    sampled = False
    with log_path.open('w') as log:
        app = subprocess.Popen(['target/release/tervy'], env=env, stdout=log, stderr=log)
        awake = subprocess.Popen(['caffeinate','-d','-i','-w',str(app.pid)])
        started = time.monotonic()
        try:
            while app.poll() is None:
                if time.monotonic() - started > 30:
                    raise TimeoutError('ten-session probe exceeded 30 s')
                if args.sample and not sampled and time.monotonic()-started > 8:
                    subprocess.run(['sample',str(app.pid),'1','-file',str(args.output.with_suffix('.sample.txt'))],timeout=3,check=False,capture_output=True)
                    sampled = True
                try:
                    tree = child_usage(app.pid)
                    samples.append({'at_s': round(time.monotonic()-started,3), 'tervy_threads': threads(app.pid), 'window': window_state(app.pid), 'tervy_mb': usage(app.pid).footprint_bytes/1e6, 'managed_children_count': len(tree), 'managed_children_mb': sum(value.footprint_bytes for value, _ in tree.values())/1e6, 'agent_cli_count':0})
                except OSError:
                    pass
                time.sleep(.2)
        except Exception as error:
            failure = str(error)
        finally:
            if app.poll() is None:
                app.kill()
                app.wait(timeout=3)
            awake.wait(timeout=3)
            # Includes leaderless members; birth times prevent SID/PID reuse.
            for sid, birth in re.findall(r'tervy-owned sid=(\d+) started_us=(\d+)', log_path.read_text()):
                result = subprocess.check_output(['target/release/examples/session_cleanup', sid, birth], text=True, timeout=4)
                cleaned.append({'sid':int(sid), 'survivors_after_app_exit':int(result.strip())})
    reports = [json.loads(line.removeprefix('tervy-many ')) for line in log_path.read_text().splitlines() if line.startswith('tervy-many ')]
    record = {'evidence':'🧪 visible macOS release UI; synthetic GPUI input; no model prompts', 'exit_code':app.returncode, 'failure':failure, 'measurements':reports, 'process_samples':samples, 'sessions':cleaned, 'note':'Configurable yes producers, one raw echo child, three visible panes, ten project switches. Frame callbacks measure GPUI frame scheduling, excluding compositor scanout. Separate managed-child trees; no agent CLI.'}
    args.output.write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({key:value for key,value in record.items() if key!='process_samples'}))
    if app.returncode or len(reports)!=1:
        raise RuntimeError(f'probe failed; cleanup finished; inspect {log_path}')
    report=reports[0]
    assert report['sessions']==args.noisy+1 and report['clean_shutdown']
    assert report['noisy_terminals']==args.noisy and report['fuzzy_matches']==3
    assert 0<=report['palette_frame_ms']<16.7
    assert report['frame_samples']>30 and report['frame_p95_ms']<33
    assert len(report['echo_samples_ms'])==20
    assert len(report['switch_samples_ms'])==10 and max(report['switch_samples_ms'])<50
    assert report['shutdown_ms']<15_000
    assert all(item['survivors_after_app_exit']==0 for item in cleaned)

if __name__=='__main__':
    main()
