#!/usr/bin/env python3
"""Compare like workloads; report >15% regressions, requiring recorded reasons.
Usage: compare.py perf/baseline-m2.json perf/baseline.json
Explanations live in candidate.regression_explanations, keyed by metric name.
Visibility changes make CPU/wakeup comparisons inconclusive, even when faster.
"""
import argparse
import json
from pathlib import Path

METRICS = {
    'startup_p50_ms': 'startup_first_frame_ms.release.p50',
    'startup_p95_ms': 'startup_first_frame_ms.release.p95',
    'idle_footprint_mb': 'idle_release.footprint_mb',
    'idle_cpu_percent': 'idle_release.cpu_percent',
    'idle_interrupt_wakeups_per_s': 'idle_release.interrupt_wakeups_per_s',
    'echo_p95_ms': 'echo.p95_painted_echo_ms',
    'write_p99_ms': 'echo.p99_pty_write_ms',
    'many_frame_p95_ms': 'many.frame_p95_ms',
    'many_palette_ms': 'many.palette_frame_ms',
    'many_echo_p95_ms': 'many.echo_p95_ms',
    'many_switch_max_ms': 'many.switch_samples_ms',
    'many_shutdown_ms': 'many.shutdown_ms',
}


def lookup(record, path):
    for key in path.split('.'):
        if not isinstance(record, dict) or key not in record:
            return None
        record = record[key]
    if path == 'many.switch_samples_ms':
        return max(record) if record else None
    return record


def compare(before, after):
    rows = []
    for name, path in METRICS.items():
        old, new = lookup(before, path), lookup(after, path)
        reason = after.get('regression_explanations', {}).get(name)
        if old is None or new is None:
            rows.append(dict(metric=name, status='MISSING', reason=reason))
            continue
        if name.startswith('many_') and lookup(before, 'many.noisy_terminals') != lookup(after, 'many.noisy_terminals'):
            rows.append(dict(metric=name, status='INCOMPARABLE', reason='Different noisy-terminal counts; compare absolute acceptance budgets instead.'))
            continue
        if name in ('idle_cpu_percent', 'idle_interrupt_wakeups_per_s') and before.get('visibility_control') != after.get('visibility_control'):
            rows.append(dict(metric=name, status='INCOMPARABLE', reason='Different window visibility control; both absolute measurements retained.'))
            continue
        change = 100 * (new / old - 1) if old else (0 if not new else None)
        regression = change is None or change > 15
        rows.append(dict(metric=name, before=old, after=new, change_percent=change,
                         status=('EXPLAINED' if reason else 'REGRESSION') if regression else 'PASS', reason=reason))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    args = parser.parse_args()
    rows = compare(json.loads(args.before.read_text()), json.loads(args.after.read_text()))
    print(json.dumps(rows, indent=2))
    if any(row['status'] in ('REGRESSION', 'MISSING') for row in rows):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
