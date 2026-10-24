#!/usr/bin/env python3
"""Guarded perf diagnostics, retaining raw records and failed scheduling attempts."""
import argparse
from typing import Any
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile

from bench import ROOT, SOURCE, GNU, gnu_command, load, read_form

EVENTS = ["instructions:u", "cycles:u", "branches:u", "branch-misses:u", "cache-references:u", "cache-misses:u"]


def collect(command, payload, events, expected):
    with tempfile.NamedTemporaryFile() as output:
        cmd = ["perf", "stat", "--no-scale", "--no-big-num", "-x", ";", "-e", ",".join(events), "-o", output.name, "--"] + command
        run = subprocess.run(cmd, input=payload, capture_output=True)
        raw = Path(output.name).read_text()
    record: dict[str, Any] = dict(command=cmd, stdout=run.stdout.decode().strip(), stderr=run.stderr.decode(), raw=raw, events={}, valid=True)
    if run.returncode or record["stdout"] != expected:
        raise RuntimeError(f"perf child failed validation: {record}")
    for line in raw.splitlines():
        fields = line.split(";")
        if len(fields) < 5 or fields[2] not in events:
            continue
        count, _, event, runtime, percentage = fields[:5]
        try:
            count, runtime, percentage = int(count), int(runtime), float(percentage)
            valid = count > 0 and runtime > 0 and percentage == 100.0
        except ValueError:
            valid = False
        record["events"][event] = dict(count=count, runtime=runtime, percent_running=percentage, valid=valid)
        record["valid"] &= valid
    record["valid"] &= set(record["events"]) == set(events)
    return record


def summarize(report):
    """Compute ratios within each simultaneous pair, not ratios of N/2N deltas.

    Cycle/cache slopes can be dominated by changing execution conditions even
    with fully scheduled counters. Preserve them as diagnostics, not IPC proof.
    """
    if not report.get("complete"):
        raise RuntimeError("cannot summarize an incomplete measurement")
    for row in report["cases"]:
        row["whole_process_2n_ratios"] = {}
        samples = [s for s in report["samples"] if s["case"] == row["case"]
                   and s["engine"] == row["engine"] and s["factor"] == 2 and s["mode"] == "pair"]
        for numerator, denominator, name in (("instructions:u", "cycles:u", "ipc"),
                ("branch-misses:u", "branches:u", "branch_miss_rate"),
                ("cache-misses:u", "cache-references:u", "cache_miss_rate")):
            values = []
            for sample in samples:
                events = sample["events"]
                if numerator not in events or denominator not in events:
                    continue
                if not sample["valid"] or any(not e["valid"] for e in events.values()):
                    raise RuntimeError("invalid pair sample in summary")
                values.append(events[numerator]["count"] / events[denominator]["count"])
            if len(values) != 3:
                raise RuntimeError("pair repetition count mismatch")
            row["whole_process_2n_ratios"][name] = dict(min=min(values), median=statistics.median(values), max=max(values))
        if "ipc" in row:
            row["slope_ipc_diagnostic_only"] = row.pop("ipc")
    report["interpretation"] = "Use within-pair whole-process ratios, with startup caveat. Cycle/cache N/2N differences are noisy diagnostics, not causal dispatch/locality proof."
    return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(ROOT / "target/release/romacs-vm"))
    parser.add_argument("--source", type=Path, default=SOURCE)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--multiplier", type=int, default=100, help="Increase N, not the body, to reduce startup noise")
    parser.add_argument("--cpu", type=int, default=min(os.sched_getaffinity(0)))
    parser.add_argument("--summarize", type=Path, help="Recompute within-pair summaries from saved real measurements")
    args = parser.parse_args()
    if args.summarize:
        report = summarize(json.loads(args.summarize.read_text()))
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps([r["whole_process_2n_ratios"] for r in report["cases"]], indent=2))
        return
    if args.multiplier < 1:
        parser.error("multiplier must be positive")
    os.sched_setaffinity(0, {args.cpu})
    report: dict[str, Any] = dict(cpu=args.cpu, multiplier=args.multiplier, perf=subprocess.check_output(["perf", "--version"], text=True).strip(),
                  policy="positive counts/runtime, 100.00% scheduling required; unscaled counts; median of 3",
                  note="Six-event runs attempted and retained, but headlines use three separately scheduled pairs. Ratios only pair simultaneous events.",
                  samples=[], cases=[])
    args.output.parent.mkdir(parents=True, exist_ok=True)

    def save():
        args.output.write_text(json.dumps(report, indent=2) + "\n")

    try:
        for name, n, body in load(args.source, "slopes").CASES:
            if name not in ("funcall", "letbind"):
                continue
            n *= args.multiplier
            for engine in ("vm2", "gnu"):
                row: dict[str, Any] = dict(case=name, engine=engine, n=n, medians={})
                for factor in (1, 2):
                    source = body.replace("N", str(n * factor))
                    command = [args.binary] if engine == "vm2" else gnu_command(source)
                    payload = read_form(source) if engine == "vm2" else b""
                    expected = subprocess.check_output(gnu_command(source), text=True).strip()
                    # Exercise the requested all-events invocation, not silently scaled results.
                    full = collect(command, payload, EVENTS, expected)
                    full.update(case=name, engine=engine, factor=factor, mode="six-at-once")
                    report["samples"].append(full)
                    save()
                    for pair in (EVENTS[:2], EVENTS[2:4], EVENTS[4:]):
                        runs = []
                        for rep in range(3):
                            sample = collect(command, payload, pair, expected)
                            sample.update(case=name, engine=engine, factor=factor, rep=rep, mode="pair")
                            report["samples"].append(sample)
                            save()
                            if not sample["valid"]:
                                raise RuntimeError(f"pair event not fully scheduled: {sample}")
                            runs.append(sample)
                        for event in pair:
                            row["medians"][f"{factor}/{event}"] = statistics.median(s["events"][event]["count"] for s in runs)
                row["slopes_per_iteration"] = {event: (row["medians"][f"2/{event}"] - row["medians"][f"1/{event}"]) / n for event in EVENTS}
                v = row["slopes_per_iteration"]
                if v["instructions:u"] <= 0 or v["cycles:u"] <= 0 or v["branches:u"] <= 0:
                    raise RuntimeError("invalid instructions/cycles/branches slope")
                row["ipc"] = v["instructions:u"] / v["cycles:u"]
                row["branch_miss_rate"] = v["branch-misses:u"] / v["branches:u"] if v["branch-misses:u"] >= 0 else None
                row["cache_miss_rate"] = v["cache-misses:u"] / v["cache-references:u"] if 0 <= v["cache-misses:u"] <= v["cache-references:u"] and v["cache-references:u"] > 0 else None
                report["cases"].append(row)
                save()
                print(json.dumps(row), flush=True)
        report["complete"] = True
        summarize(report)
    except BaseException as e:
        report["complete"] = False
        report["error"] = str(e)
        raise
    finally:
        save()


if __name__ == "__main__":
    main()
