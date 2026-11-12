#!/usr/bin/env python3
"""Fail closed if the pinned optimized VM calls, rather than jumps, to handlers.

This checks generated x86-64 code, not a Rust language guarantee. Run alongside
long-chain debug/release tests after compiler/flags/handler changes. It does not
prove memory safety, semantics, or behavior on another target architecture.

Every function of the engine's module tree is a handler here, nested modules
included: the typed handlers' entries are `threaded::audited::t_*`, each the
aligned wrapper around an inlined body (threaded_hot.rs), whose dispatch must
stay a jump.
"""
import argparse
import json
import re
import subprocess
from pathlib import Path

HANDLER = re.compile(r"romacs_vm::threaded::((?:\w+::)*\w+)>:")
REQUIRED = {"add", "sub", "mul", "inc", "dec", "less", "equal", "ret", "call", "direct", "cons",
            "primitive", "cold", "resolve_entry", "enter_large", "grow_cons", "apply",
            "audited::legacy", "audited::t_mov", "audited::t_mov_wide", "audited::t_constant",
            "audited::t_constant_wide", "audited::t_unless", "audited::t_unless_wide", "audited::t_jump",
            "audited::t_car", "audited::t_cdr", "audited::t_add", "audited::t_sub", "audited::t_inc",
            "audited::t_dec", "audited::t_less", "audited::t_greater", "audited::t_equal",
            "audited::t_null", "audited::t_consp", "audited::t_eq"}


def scan(text):
    """(records, violations) for the objdump -Cd TEXT: one record per engine
    function, and each call that is not a linkage stub's."""
    records, violations = [], []
    for function in re.split(r"\n[0-9a-f]+ <", "\n" + text):
        header, _, body = function.partition("\n")
        match = HANDLER.fullmatch(header)
        if not match or match[1] == "execute":
            continue
        calls = [line for line in body.splitlines() if re.search(r"\bcall\s", line)]
        for line in calls:
            # Register-indirect calls are the dispatch hazard. RIP-relative
            # calls go through linkage stubs (allocator/memcpy/etc.), not pc.
            if ("*" in line and "%rip" not in line) or "<romacs_vm::threaded::" in line:
                violations.append(dict(handler=match[1], instruction=line.strip()))
        records.append(dict(handler=match[1], calls=len(calls),
                            indirect_jumps=len(re.findall(r"\bjmp\s+\*", body))))
    return records, violations


def check(text):
    """The report for the objdump -Cd TEXT: its records, and its violations
    with the required handlers it lacks."""
    records, violations = scan(text)
    missing = REQUIRED - {r["handler"] for r in records}
    if missing:
        violations.append(dict(missing=sorted(missing)))
    return dict(target="x86-64", handlers=records, violations=violations, passed=not violations)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    report = dict(binary=str(args.binary.resolve()),
                  **check(subprocess.check_output(["objdump", "-Cd", str(args.binary)], text=True)))
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    if report["violations"]:
        raise SystemExit("handler dispatch is not bounded: inspect generated code")


if __name__ == "__main__":
    main()
