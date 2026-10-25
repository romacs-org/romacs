#!/usr/bin/env python3
"""Run targeted reverse semantic mutants; require named behavioral mismatches."""
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "crates/romacs-vm/src/prims/reverse.rs"
MUTANTS = [
    ("cycle-datum", "Fault::Circular(tail)", "Fault::Circular(seq)", "REVERSE_ORACLE_lasso"),
    ("properties", "cx.make_string(bytes, multibyte)", "cx.buffer_name_string(seq, bytes, multibyte)", "REVERSE_ORACLE_properties"),
    ("list-car", "cx.cons(car, result)", "cx.cons(cdr, result)", "REVERSE_ORACLE_list-copy"),
    ("vector-order", "values.reverse();", "// values.reverse();", "REVERSE_ORACLE_vector"),
    ("ascii-kind", "let multibyte = !one_byte && text.multibyte;", "let multibyte = text.multibyte;", "REVERSE_ORACLE_multibyte-ascii"),
    ("dotted-datum", 'Fault::Wrong("listp", tail)', 'Fault::Wrong("listp", seq)', "REVERSE_ORACLE_dotted"),
    ("wrong-predicate", 'Fault::Wrong("sequencep", seq)', 'Fault::Wrong("listp", seq)', "REVERSE_ORACLE_wrong"),
    ("byte-order", "text.bytes.iter().rev().copied().collect()", "text.bytes.iter().copied().collect()", "REVERSE_ORACLE_unibyte"),
    ("character-order", "chars.into_iter().rev()", "chars.into_iter()", "REVERSE_ORACLE_extended"),
]


def run():
    return subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", "reverse", "reverse_matches_pinned_gnu", "--", "--nocapture"], cwd=ROOT, capture_output=True, text=True, timeout=180)


if __name__ == "__main__":
    original = SOURCE.read_text()
    rows = []
    logs = Path(tempfile.gettempdir()) / "reverseboot-evidence"
    logs.mkdir(exist_ok=True)
    try:
        for name, old, new, key in MUTANTS:
            assert original.count(old) == 1, name
            SOURCE.write_text(original.replace(old, new))
            result = run()
            output = result.stdout + result.stderr
            (logs / f"{name}.log").write_text(output)
            matched = [line for line in output.splitlines() if line.startswith(key + ":")]
            assert result.returncode != 0 and matched and "could not compile" not in output, (name, output)
            rows.append(dict(mutant=name, exit=result.returncode, assertion=key, mismatch=matched[0]))
            print(name, "KILLED", key, flush=True)
    finally:
        SOURCE.write_text(original)
    result = run()
    (logs / "restored-green.log").write_text(result.stdout + result.stderr)
    assert result.returncode == 0, result.stderr
    (logs / "results.json").write_text(json.dumps(rows, indent=2) + "\n")
    print("Restored production source: green")
