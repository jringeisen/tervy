#!/usr/bin/env python3
"""Measure DevDesk alone using macOS cumulative process counters.

Build first, then run: scripts/perf/idle.py --layout normal --seconds 60
The sampler does no recurring work in DevDesk. Counter layout/units come from
the selected macOS SDK's sys/resource.h (rusage_info_v0). CPU Mach ticks are
converted with mach_timebase_info; see XNU task_power_info_locked in task.c.
Package-idle wakeups are not all thread wakeups and do not identify timers.
"""

import argparse
import ast
import ctypes
import json
import os
from pathlib import Path
import subprocess
import re
import time


class Usage(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16)] + [
        (name, ctypes.c_uint64)
        for name in (
            "user_ticks", "system_ticks", "idle_wakeups", "interrupt_wakeups",
            "pageins", "wired_bytes", "resident_bytes", "footprint_bytes",
            "start", "exit",
        )
    ]


def usage(pid):
    value = Usage()
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    libproc.proc_pid_rusage.restype = ctypes.c_int
    if libproc.proc_pid_rusage(pid, 0, ctypes.byref(value)):
        raise OSError(ctypes.get_errno(), "proc_pid_rusage failed")
    return value


def child_usage(root_pid):
    """External process-tree snapshot; never runs inside DevDesk."""
    rows = subprocess.check_output(["ps", "-ax", "-o", "pid=,ppid=,comm="], text=True)
    children = []
    for line in rows.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) == 3:
            children.append((int(fields[0]), int(fields[1]), fields[2]))
    descendants = {root_pid}
    while True:
        expanded = descendants | {pid for pid, parent, _ in children if parent in descendants}
        if expanded == descendants:
            break
        descendants = expanded
    result = {}
    for pid, _, command in children:
        if pid in descendants and pid != root_pid:
            try:
                result[pid] = (usage(pid), command)
            except OSError:
                pass
    return result


def nanoseconds_per_tick():
    class Timebase(ctypes.Structure):
        _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]

    info = Timebase()
    system = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
    system.mach_timebase_info.argtypes = [ctypes.POINTER(Timebase)]
    if system.mach_timebase_info(ctypes.byref(info)) or not info.denom:
        raise RuntimeError("mach_timebase_info failed")
    return info.numer / info.denom


def threads(pid):
    # SDK sys/proc_info.h, proc_taskinfo: six uint64s then twelve int32s.
    class Task(ctypes.Structure):
        _fields_ = [("times", ctypes.c_uint64 * 6), ("counts", ctypes.c_int32 * 12)]
    value = Task()
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib")
    if libproc.proc_pidinfo(pid, 4, 0, ctypes.byref(value), ctypes.sizeof(value)) != ctypes.sizeof(value):
        raise OSError("proc_pidinfo task failed")
    return value.counts[9]


def window_state(pid):
    helper = Path("target/perf-helpers/window-state")
    if not helper.exists():
        return {"verified": False, "reason": "build scripts/perf/window-state.swift first"}
    return json.loads(subprocess.check_output([str(helper), str(pid)], text=True, timeout=3))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--layout", choices=["normal", "no-sidebar", "minimal", "terminal", "idle20", "scrollback20"], default="normal")
    parser.add_argument("--seconds", type=float, default=60)
    parser.add_argument("--settle", type=float, default=10)
    parser.add_argument("--binary", type=Path, default=Path("target/release/devdesk"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--sample", action="store_true")
    args = parser.parse_args()
    if args.seconds <= 0 or args.settle < 0:
        parser.error("seconds must be positive and settle nonnegative")
    env = os.environ.copy()
    ns_per_tick = nanoseconds_per_tick()
    env["DEVDESK_PERF_LAYOUT"] = args.layout
    if args.layout == "scrollback20":
        env["DEVDESK_PERF_SCENARIO"] = "scrollback"
    env["DEVDESK_LOG"] = "devdesk=trace"
    # Keep the first-frame benchmark's quit flag out of long-running probes.
    env.pop("DEVDESK_PERF_EXIT", None)
    env.pop("DEVDESK_PERF_T0", None)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.with_suffix(".stderr.txt").open("w") as log:
        app = subprocess.Popen([str(args.binary.resolve())], env=env, stdout=log, stderr=log)
        awake = subprocess.Popen(["caffeinate", "-d", "-i", "-w", str(app.pid)])
        try:
            print(f"{args.layout}: pid {app.pid}, settling {args.settle}s", flush=True)
            time.sleep(args.settle)
            grids = []
            if args.layout in ("idle20", "scrollback20"):
                lines = args.output.with_suffix(".stderr.txt").read_text().splitlines()
                reports = [line.removeprefix("devdesk-idle-grids ") for line in lines if line.startswith("devdesk-idle-grids ")]
                if len(reports) != 1:
                    raise RuntimeError("missing one-shot grid report before idle interval")
                grids = ast.literal_eval(reports[0])
                if len(grids) != 20 or (args.layout == "scrollback20" and any(row[2] != 5000 for row in grids)):
                    raise RuntimeError(f"invalid 20-session scrollback fixture: {grids}")
            initial_window = window_state(app.pid)
            first = usage(app.pid)
            first_children = child_usage(app.pid)
            start = time.monotonic()
            time.sleep(args.seconds)
            last = usage(app.pid)
            elapsed = time.monotonic() - start
            last_children = child_usage(app.pid)
            common = first_children.keys() & last_children.keys()
            child_ticks = sum((last_children[pid][0].user_ticks + last_children[pid][0].system_ticks)
                              - (first_children[pid][0].user_ticks + first_children[pid][0].system_ticks) for pid in common)
            result = {
                "layout": args.layout,
                "terminal_grids_columns_rows_history": grids,
                "window_before": initial_window,
                "window_after": window_state(app.pid),
                "threads_at_end": threads(app.pid),
                "managed_children": {
                    "count_at_end": len(last_children),
                    "footprint_sum_mb": sum(value.footprint_bytes for value, _ in last_children.values()) / 1_000_000,
                    "cpu_percent_common_pids": 100 * child_ticks * ns_per_tick / (elapsed * 1e9),
                    "same_pids_entire_interval": first_children.keys() == last_children.keys(),
                },
                "agent_cli_processes": {"count": 0, "footprint_sum_mb": 0, "cpu_percent": 0},
                "settle_s": args.settle,
                "elapsed_s": elapsed,
                "cpu_percent": 100 * ((last.user_ticks + last.system_ticks) - (first.user_ticks + first.system_ticks)) * ns_per_tick / (elapsed * 1e9),
                "cpu_tick_delta": (last.user_ticks + last.system_ticks) - (first.user_ticks + first.system_ticks),
                "nanoseconds_per_cpu_tick": ns_per_tick,
                "footprint_mb": last.footprint_bytes / 1_000_000,
                "package_idle_wakeups_per_s": (last.idle_wakeups - first.idle_wakeups) / elapsed,
                "interrupt_wakeups_per_s": (last.interrupt_wakeups - first.interrupt_wakeups) / elapsed,
                "method": "macOS proc_pid_rusage RUSAGE_INFO_V0; deltas / monotonic elapsed time; decimal MB",
            }
            if args.layout in ("idle20", "scrollback20") and (len(first_children) != 20 or len(last_children) != 20):
                raise RuntimeError("20-session fixture lost a worker")
            args.output.write_text(json.dumps(result, indent=2) + "\n")
            print(json.dumps(result), flush=True)
            if args.sample:
                subprocess.run(["sample", str(app.pid), "3", "-file", str(args.output.with_suffix(".sample.txt"))], check=True)
        finally:
            app.terminate()
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait()
            awake.wait(timeout=3)
            for sid, birth in re.findall(r"devdesk-owned sid=(\d+) started_us=(\d+)", args.output.with_suffix(".stderr.txt").read_text()):
                subprocess.run(["target/release/examples/session_cleanup", sid, birth], check=True, capture_output=True, timeout=5)


if __name__ == "__main__":
    main()
