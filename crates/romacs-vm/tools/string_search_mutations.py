#!/usr/bin/env python3
"""Run bounded compiling string-search mutants; restore source after each."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--out', type=Path, required=True)
a = parser.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
root = Path(__file__).resolve().parents[3]
source = root / 'crates/romacs-vm/src/prims/search.rs'
mutants = [
    ('byte-index', 'Value::integer(character as i64)', 'Value::integer(byte as i64)'),
    ('ignore-start', 'let start = start as usize;', 'let start = 0usize;'),
    ('empty-needle', 'Some(start_byte) } else {', 'None } else {'),
    ('skip-widening', 'haystack.is_multibyte() && !needle.is_multibyte() && !pattern.is_ascii()', 'false && haystack.is_multibyte() && !needle.is_multibyte() && !pattern.is_ascii()'),
    ('skip-narrowing', '} else if !haystack.is_multibyte() && needle.is_multibyte() {', '} else if false && !haystack.is_multibyte() && needle.is_multibyte() {'),
]
rows = []
for name, old, new in mutants:
    original = source.read_bytes()
    text = original.decode()
    assert text.count(old) == 1, name
    try:
        source.write_text(text.replace(old, new))
        cmd = ['cargo', 'test', '-p', 'romacs-vm', '--jobs', '4', '--test', 'string_search', '--', '--nocapture']
        run = subprocess.run(cmd, cwd=root, capture_output=True, timeout=300)
        output = run.stdout + run.stderr
        (a.out / (name + '.log')).write_bytes(output)
        killed = run.returncode == 101 and b'assertion `left == right` failed: STRING_SEARCH_GNU' in output and b'test result: FAILED' in output
        rows.append(dict(name=name, command=cmd, exit=run.returncode, assertion_kill=killed, source_sha256=hashlib.sha256(original).hexdigest(), log_sha256=hashlib.sha256(output).hexdigest()))
    finally:
        source.write_bytes(original)
    rows[-1]['restored'] = source.read_bytes() == original
    (a.out / 'receipt.json').write_text(json.dumps(rows, indent=2) + '\n')
    print(rows[-1], flush=True)
    assert killed and rows[-1]['restored'], name
