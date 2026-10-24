"""Tier P: counters exact or deterministic within a measured resolution.

Instruction resolution is 36 instructions; other counters are exact.
Differences within that resolution are not evidence. A probe whose observed
variation exceeds its resolution does not belong here: --qualify disqualifies
it, retaining every reading rather than selecting or correcting samples.
Normal measurements run once, with no retry, quiet window or sample selection.
Unavailable or multiplexed instruction events are refused. Wall time is never
a probe value.

Adding a probe is one function and one registration:

    @probe("name", "unit", answer=KNOWN_OR_NONE)
    def name():
        \"\"\"One line saying what is counted and why.\"\"\"
        return count

Rust probes live in scripts/probe-rs/src/main.rs (one function, one line in
PROBES) and are listed here automatically. `scripts/probe` is the CLI.
"""
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass
from typing import Callable

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
RUST = HERE / "probe-rs"
INSTRUCTION_RESOLUTION = 36


@dataclass(frozen=True)
class Probe:
    name: str
    unit: str
    answer: int | None
    doc: str
    run: Callable[[], int]


def resolution(entry):
    """Largest non-evidential instruction difference; other counters are exact."""
    return INSTRUCTION_RESOLUTION if entry.unit == "instructions:u" else 0


def matches_answer(entry, value):
    """Known answers check counter sanity within resolution, not hardware exactness."""
    return None if entry.answer is None else abs(value - entry.answer) <= resolution(entry)


def qualifies(entry, values):
    """Disqualify observed run-to-run spread beyond this tier's resolution."""
    return bool(values) and max(values) - min(values) <= resolution(entry)


PYTHON_PROBES: dict[str, Probe] = {}


def probe(name, unit, answer=None):
    """Register a Python probe returning one integer."""
    def register(function):
        if name in PYTHON_PROBES:
            raise ValueError(f"duplicate probe: {name}")
        doc = (function.__doc__ or "").strip().splitlines()[0] if function.__doc__ else ""
        PYTHON_PROBES[name] = Probe(name, unit, answer, doc, function)
        return function
    return register


# ------------------------------------------------------------ Rust probes

def cargo():
    explicit = os.environ.get("ROMACS_PROBE_CARGO")
    if explicit:
        return explicit
    found = shutil.which("cargo")
    if found:
        return found
    channel = tomllib.loads((REPO / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    pinned = sorted(Path("/nix/store").glob(f"*-rust-default-{channel}/bin/cargo"))
    if pinned:
        return str(pinned[-1])
    raise RuntimeError(f"no cargo: set ROMACS_PROBE_CARGO or put Rust {channel} on PATH")


def rust_binary():
    """Build (a no-op when fresh) and return the Rust probe binary."""
    target = REPO / "target" / "probe-rs"
    command = [cargo(), "build", "--release", "--locked", "--quiet",
               "--manifest-path", str(RUST / "Cargo.toml"), "--target-dir", str(target)]
    env = dict(os.environ, PATH=f"{Path(command[0]).parent}{os.pathsep}{os.environ.get('PATH', '')}")
    built = subprocess.run(command, env=env, capture_output=True, text=True)
    if built.returncode:
        raise RuntimeError(f"building {RUST.name} failed:\n{built.stderr}")
    return target / "release" / "romacs-probe"


def rust_probes(binary):
    listed = subprocess.run([binary, "--list"], capture_output=True, text=True, check=True).stdout
    probes = {}
    for line in listed.splitlines():
        name, unit, answer, doc = line.split("\t")

        def run(name=name):
            done = subprocess.run([binary, name], capture_output=True, text=True)
            if done.returncode:
                raise RuntimeError(done.stderr.strip() or f"{name} exited {done.returncode}")
            return int(done.stdout)
        probes[name] = Probe(name, unit, int(answer) if answer else None, doc, run)
    return probes


# --------------------------------------------------------------------- CLI

def registry(names=()):
    """Python probes, plus the Rust ones unless every requested name is Python."""
    probes = dict(PYTHON_PROBES)
    if not names or any(name not in probes for name in names):
        for name, rust in rust_probes(rust_binary()).items():
            if name in probes:
                raise ValueError(f"probe registered twice: {name}")
            probes[name] = rust
    return probes


def run(names, probes, *, as_json=False, out=sys.stdout, clock=time.monotonic, setup_seconds=0.0, baseline=None):
    """Run named probes (all when empty). Exit 0 ok, 1 known answer missed, 2 error.

    setup_seconds (building and listing the Rust probes) is added to the total.
    """
    unknown = [name for name in names if name not in probes]
    if unknown:
        print(f"probe: unknown probe(s): {', '.join(unknown)}; see --list", file=sys.stderr)
        return 2
    results, status, started = [], 0, clock()
    for name in names or list(probes):
        entry = probes[name]
        begin = clock()
        try:
            value = entry.run()
        except Exception as error:  # report the probe, keep running the rest
            print(f"probe: {name}: {error}", file=sys.stderr)
            results.append({"name": name, "error": str(error), "seconds": clock() - begin})
            status = 2
            continue
        if type(value) is not int:
            print(f"probe: {name} returned {value!r}, not an integer", file=sys.stderr)
            results.append({"name": name, "error": "non-integer value", "seconds": clock() - begin})
            status = 2
            continue
        matches = matches_answer(entry, value)
        if matches is False:
            status = max(status, 1)
        results.append({"name": name, "value": value, "unit": entry.unit, "answer": entry.answer,
                        "matches": matches, "resolution": resolution(entry), "seconds": clock() - begin})
        if baseline is not None:
            before = baseline[name]
            if before["unit"] != entry.unit or type(before["value"]) is not int:
                raise ValueError(f"invalid baseline for {name}")
            delta = value - before["value"]
            results[-1].update(delta=delta, evidence=abs(delta) > resolution(entry))
        if not as_json:
            check = ("" if matches is None else "  = known answer" if matches
                     else f"  MISMATCH: known answer {entry.answer}")
            print(f"{name:34} {value} {entry.unit}{check}  ({results[-1]['seconds']:.2f}s)", file=out)
        if not as_json and baseline is not None:
            print(f"  delta {delta:+d}; resolution {resolution(entry)}; "
                  f"{'difference' if results[-1]['evidence'] else 'not evidence'}", file=out)
    total = clock() - started + setup_seconds
    if as_json:
        json.dump({"probes": results, "setup_seconds": setup_seconds, "seconds": total}, out)
        out.write("\n")
    else:
        print(f"{len(results)} probes in {total:.2f}s (build and registry {setup_seconds:.2f}s)", file=out)
    return status


def qualify(names, probes, samples, *, out=sys.stdout):
    """Audit variation without producing a selected measurement or retrying failures."""
    if samples < 50:
        raise ValueError("qualification requires at least 50 readings")
    results, status = [], 0
    for name in names or list(probes):
        entry = probes[name]
        values = []
        errors = []
        for _ in range(samples):
            try:
                value = entry.run()
                if type(value) is not int:
                    raise ValueError("non-integer value")
                values.append(value)
            except Exception as error:
                errors.append(str(error))
        eligible = not errors and qualifies(entry, values)
        answers_ok = all(matches_answer(entry, value) is not False for value in values)
        if not eligible:
            status = 2
        elif not answers_ok:
            status = max(status, 1)
        results.append(dict(name=name, values=values, errors=errors,
                            spread=max(values) - min(values) if values else None,
                            resolution=resolution(entry), qualified=eligible, matches=answers_ok))
    json.dump({"qualification": results}, out)
    out.write("\n")
    return status


def list_probes(probes, out=sys.stdout):
    for entry in probes.values():
        answer = "" if entry.answer is None else f"  [known answer {entry.answer}]"
        print(f"{entry.name:34} {entry.unit}{answer}\n{'':34} {entry.doc}", file=out)


def main(argv=None):
    import argparse
    parser = argparse.ArgumentParser(
        prog="probe", description="Tier P: counters in seconds, exact or within stated resolution.",
        epilog="examples: scripts/probe --list | scripts/probe | scripts/probe mem-live-conses")
    parser.add_argument("names", nargs="*", metavar="NAME", help="probes to run (default: all)")
    parser.add_argument("--list", action="store_true", help="show every probe, its unit and known answer")
    parser.add_argument("--json", action="store_true", help="one JSON object on stdout")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--qualify", type=int, metavar="N", help="audit at least 50 readings; disqualify excess variation")
    mode.add_argument("--compare", type=Path, metavar="JSON", help="compare against a previous --json run; suppress sub-resolution evidence")
    args = parser.parse_args(argv)
    started = time.monotonic()
    try:
        probes = registry(args.names)
    except (RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        print(f"probe: {error}", file=sys.stderr)
        return 2
    setup = time.monotonic() - started
    if args.list:
        list_probes(probes)
        return 0
    try:
        if args.qualify is not None:
            if any(name not in probes for name in args.names):
                raise ValueError("unknown probe in qualification")
            return qualify(args.names, probes, args.qualify)
        baseline = None
        if args.compare is not None:
            baseline = {row["name"]: row for row in json.loads(args.compare.read_text())["probes"]}
            for name in args.names or list(probes):
                row = baseline[name]
                if row["unit"] != probes[name].unit or type(row["value"]) is not int:
                    raise ValueError(f"invalid baseline for {name}")
        return run(args.names, probes, as_json=args.json, setup_seconds=setup, baseline=baseline)
    except (OSError, KeyError, ValueError) as error:
        print(f"probe: {error}", file=sys.stderr)
        return 2
