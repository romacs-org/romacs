#!/usr/bin/env python3
"""Kill logior mutants only at BITWISE_GNU_MISMATCH and the named oracle row.
Logs must be outside the worktree. Every mutation is restored even on failure.
"""
import pathlib
import subprocess
import sys
ROOT = pathlib.Path(__file__).resolve().parents[5]
LOG = pathlib.Path(sys.argv[1]).resolve()
assert not LOG.is_relative_to(ROOT), "keep run evidence outside the worktree"
LOG.mkdir(parents=True, exist_ok=True)
SOURCE = ROOT / "crates/romacs-vm/src/prims/bitwise.rs"
original = SOURCE.read_text()
mutants = [
    ("absent", 'name = "logior"', 'name = "logior-absent"', "logior-0"),
    ("empty-identity", 'BigInt::from(0)', 'BigInt::from(1)', "logior-0"),
    ("or-to-xor", 'result |=', 'result ^=', "logior-28"),
    ("first-type", 'if first || cx.float_value(value).is_ok()', 'if cx.float_value(value).is_ok()', "logior-10"),
    ("later-type", 'if first || cx.float_value(value).is_ok()', 'if true || first || cx.float_value(value).is_ok()', "logior-11"),
    ("later-float", 'first || cx.float_value(value).is_ok()', 'first', "logior-12"),
    ("marker-position", '.map(Into::into)', '.map(|_| num_bigint::BigInt::from(0))', "logior-16"),
    ("unset-marker", 'Marker does not point anywhere', 'Wrong marker', "logior-18"),
    ("single-identity", 'if args.len() == 1 &&', 'if false && args.len() == 1 &&', "logior-19"),
    ("narrow-bignum", 'Ok(cx.bigint(result))', 'Ok(cx.bigint(num_bigint::BigInt::from(i64::try_from(result).unwrap_or(0))))', "logior-5"),
]
try:
    for name, old, new, row in mutants:
        assert original.count(old) == 1, (name, old)
        mutated = original.replace(old, new)
        assert mutated != original
        SOURCE.write_text(mutated)
        run = subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", "bitwise", "bitwise_matches_pinned_gnu", "--", "--nocapture"], cwd=ROOT, capture_output=True, text=True)
        output = run.stdout + run.stderr
        (LOG / (name + ".log")).write_text(output)
        killed = run.returncode != 0 and "BITWISE_GNU_MISMATCH" in output and f'"{row}": GNU' in output and "ours" in output
        print(f"{name}: {'KILLED-AT-TARGET' if killed else 'INVALID-OR-SURVIVED'} rc={run.returncode} assertion=BITWISE_GNU_MISMATCH row={row}", flush=True)
        assert killed, name
        SOURCE.write_text(original)
finally:
    SOURCE.write_text(original)
