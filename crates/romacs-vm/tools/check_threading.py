#!/usr/bin/env python3
"""Fail closed if the pinned optimized VM calls, rather than jumps, to handlers.

This checks generated x86-64 code, not a Rust language guarantee. Run alongside
long-chain debug/release tests after compiler/flags/handler changes. It does not
prove memory safety, semantics, or behavior on another target architecture.
"""
import argparse
import json
import re
import subprocess
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    text = subprocess.check_output(["objdump", "-Cd", str(args.binary)], text=True)
    functions = re.split(r"\n[0-9a-f]+ <", "\n" + text)
    records = []
    violations = []
    for function in functions:
        header, _, body = function.partition("\n")
        match = re.fullmatch(r"romacs_vm::threaded::(\w+)>:", header)
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
    names = {r["handler"] for r in records}
    required = {"mov", "constant", "add", "sub", "mul", "inc", "dec", "less",
                "equal", "jump", "unless", "ret", "call", "direct", "cons",
                "primitive", "cold", "resolve_entry", "enter_large", "grow_cons", "apply"}
    if not required <= names:
        violations.append(dict(missing=sorted(required - names)))
    report = dict(binary=str(args.binary.resolve()), target="x86-64", handlers=records,
                  violations=violations, passed=not violations)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    if violations:
        raise SystemExit("handler dispatch is not bounded: inspect generated code")


if __name__ == "__main__":
    main()
