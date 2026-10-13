#!/usr/bin/env python3
"""Opt-in UI benchmarks. No model prompt is sent in the agent scenarios.

Build release first. This external sampler records footprint; Tervy reports
paint timings. Echo dispatches synthetic GPUI keystrokes through native text input; timings
exclude physical keyboard delivery and compositor scanout. Only scenario names are accepted as commands.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
from idle import usage, child_usage


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", choices=["echo", "flood", "seq", "claude", "codex"])
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=30)
    args = parser.parse_args()
    flood = Path("/private/tmp/tervy-perf-flood.txt")
    if args.scenario == "flood":
        # Exactly 100 decimal MB of printable lines; use cat rather than zero
        # bytes, which would benchmark the parser's control-byte fast path.
        line = b"flood " + b"x" * 93 + b"\n"
        with flood.open("wb") as file:
            block = (line * 10000)
            remaining = 100_000_000
            while remaining:
                data = block[:remaining]
                file.write(data)
                remaining -= len(data)
    env = dict(os.environ, TERVY_PERF_LAYOUT="terminal", TERVY_PERF_SCENARIO=args.scenario)
    env.pop("TERVY_PERF_EXIT", None)
    env.pop("TERVY_PERF_T0", None)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    samples = []
    children = []
    last_child_sample = 0
    with args.output.with_suffix(".stderr.txt").open("w") as log:
        child = subprocess.Popen([str(Path("target/release/tervy").resolve())], env=env, stdout=log, stderr=log)
        start = time.monotonic()
        try:
            while child.poll() is None:
                if time.monotonic() - start > args.timeout:
                    raise TimeoutError(f"{args.scenario} did not finish in {args.timeout}s")
                try:
                    samples.append({"at_s": round(time.monotonic() - start, 3), "footprint_mb": round(usage(child.pid).footprint_bytes / 1_000_000, 3)})
                except OSError:
                    pass  # Process can exit between poll and proc_pid_rusage.
                if time.monotonic() - last_child_sample >= .2:
                    tree = child_usage(child.pid)
                    # Early startup may contain only the environment resolver.
                    commands = [command for _, command in tree.values()]
                    agent_present = any(Path(command).name in ["claude", "codex"] for command in commands)
                    children.append({"at_s": round(time.monotonic()-start, 3), "count": len(tree),
                                     "footprint_sum_mb": round(sum(value.footprint_bytes for value, _ in tree.values())/1_000_000, 3),
                                     "agent_present": agent_present, "commands": commands})
                    last_child_sample = time.monotonic()
                time.sleep(.05)
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=2)
            if args.scenario == "flood":
                flood.unlink(missing_ok=True)
    measurements = []
    for line in args.output.with_suffix(".stderr.txt").read_text().splitlines():
        if line.startswith("tervy-terminal-perf "):
            measurements.append(json.loads(line.removeprefix("tervy-terminal-perf ")))
    if not measurements or (args.scenario in ["echo", "flood", "seq"] and len(measurements) < 2):
        raise RuntimeError("benchmark did not report completion; inspect stderr log")
    agent = args.scenario in ["claude", "codex"]
    agent_samples = [sample for sample in children if sample["agent_present"]] if agent else []
    if agent and not agent_samples:
        raise RuntimeError("no agent process observed; inspect the CLI command and sampler")
    result = {"scenario": args.scenario,
              "managed_children": {"samples": children if not agent else [], "note": "external snapshots; sums are distinct from Tervy"},
              "agent_cli_processes": {"samples": agent_samples, "peak_sampled_footprint_sum_mb": max((sample["footprint_sum_mb"] for sample in agent_samples), default=0)}, "exit_code": child.returncode, "measurements": measurements,
              "peak_sampled_footprint_mb": max(sample["footprint_mb"] for sample in samples), "footprint_samples": samples,
              "note": "opt-in visible release terminal; first content/echo means GPUI paint, not compositor scanout or agent-ready state"}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: value for key, value in result.items() if key not in ["footprint_samples", "managed_children", "agent_cli_processes"]}))

if __name__ == "__main__":
    main()
