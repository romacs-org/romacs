#!/usr/bin/env python3
"""Prove exact GNU logand assertions reject targeted behavioral mutations."""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[5]
LOG = pathlib.Path(sys.argv[1]).resolve()
assert not LOG.is_relative_to(ROOT)
LOG.mkdir(parents=True, exist_ok=True)
SOURCE = ROOT / "crates/romacs-vm/src/prims/bitwise.rs"
original = SOURCE.read_text()
head, tail = original.split('defsubr! { name = "logand"', 1)
mutants = [
    ("absent", "tail", 'gnu = "data.c:3494"', 'gnu = "data.c:3494"', "logand-0"),
    ("empty-identity", "tail", 'BigInt::from(-1)', 'BigInt::from(0)', "logand-0"),
    ("and-to-or", "tail", 'result &=', 'result |=', "logand-28"),
    ("and-to-xor", "tail", 'result &=', 'result ^=', "logand-28"),
    ("first-type", "head", 'if first || cx.float_value(value).is_ok()', 'if cx.float_value(value).is_ok()', "logand-10"),
    ("later-type", "head", 'if first || cx.float_value(value).is_ok()', 'if true || first || cx.float_value(value).is_ok()', "logand-11"),
    ("later-float", "head", 'first || cx.float_value(value).is_ok()', 'first', "logand-12"),
    ("marker-position", "head", '.map(Into::into)', '.map(|_| num_bigint::BigInt::from(0))', "logand-39"),
    ("unset-marker", "head", 'Marker does not point anywhere', 'Wrong marker', "logand-18"),
    ("single-identity", "tail", 'if args.len() == 1 &&', 'if false && args.len() == 1 &&', "logand-19"),
    ("narrow-bignum", "tail", 'Ok(cx.bigint(result))', 'Ok(cx.bigint(num_bigint::BigInt::from(i64::try_from(result).unwrap_or(0))))', "logand-5"),
]
results = []
try:
    for name, part, old, new, row in mutants:
        source = head if part == "head" else tail
        assert source.count(old) == 1, (name, old)
        replacement = source.replace(old, new)
        mutated = replacement + 'defsubr! { name = "logand"' + tail if part == "head" else head + 'defsubr! { name = "logand"' + replacement
        if name == "absent":
            assert original.count('name = "logand"') == 1
            mutated = original.replace('name = "logand"', 'name = "logand-absent"')
        assert mutated != original
        SOURCE.write_text(mutated)
        run = subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", "logand", "logand_matches_pinned_gnu", "--", "--nocapture"], cwd=ROOT, capture_output=True, text=True)
        output = run.stdout + run.stderr
        (LOG / (name + ".log")).write_text(output)
        killed = run.returncode != 0 and "LOGAND_GNU_MISMATCH" in output and f'"{row}": GNU' in output
        result = dict(mutant=name, killed_at_target=killed, returncode=run.returncode, row=row)
        results.append(result)
        print(json.dumps(result), flush=True)
        assert killed, name
        SOURCE.write_text(original)
finally:
    SOURCE.write_text(original)
    (LOG / "results.json").write_text(json.dumps(results, indent=2) + "\n")
