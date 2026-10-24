#!/usr/bin/env python3
"""Body mutants: every rejection must reach a runtime assertion, not compilation."""
from pathlib import Path
import json
import subprocess

root = Path(__file__).resolve().parents[4]
source = root / 'crates/romacs-vm/src/prims/number_parse.rs'
logs = Path(__file__).resolve().parent / 'logs'
original = source.read_text()
mutants = [
    ('radix-ignored', 'n as u32', '10u32', False),
    ('newline-whitespace', "Some(b' ' | b'\\t')", "Some(b' ' | b'\\t' | b'\\n')", False),
    ('float-disabled', 'if trailing || (leading && exponent) {', 'if false && (trailing || (leading && exponent)) {', False),
    ('range-wrong-condition', 'return Err(Fault::Range1(base));', 'return Err(Fault::Wrong("fixnump", base));', False),
    ('sign-lost', 'if negative { -(magnitude as i64) } else { magnitude as i64 }', 'if negative { magnitude as i64 } else { magnitude as i64 }', False),
    ('overflow-silent-zero', 'return Err(Fault::Error("string-to-number: bignum results are not supported".into()));', 'return Ok(Value::integer(0).unwrap());', False),
    ('nan-payload-lost', '(payload & 0x0007_ffff_ffff_ffff)', '(payload & 0)', True),
    ('nan-dot-sentinel-lost', 'let payload = if leading { magnitude } else { u64::MAX - 1 };', 'let payload = magnitude;', True),
]
summary = []
try:
    for name, before, after, unit in mutants:
        assert original.count(before) == 1, (name, before)
        source.write_text(original.replace(before, after))
        command = ['cargo', 'test', '-p', 'romacs-vm']
        command += ['--lib', 'string_to_number_preserves_nan_payload_bits_before_printing'] if unit else ['--test', 'string_to_number']
        result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (logs / f'string-to-number-mutant-{name}.log').write_text(result.stdout)
        killed = result.returncode != 0 and 'assertion `left == right` failed' in result.stdout and 'test result: FAILED' in result.stdout
        summary.append({'mutant': name, 'returncode': result.returncode, 'assertion_killed': killed})
        print(summary[-1], flush=True)
        assert killed, name
        source.write_text(original)
finally:
    source.write_text(original)
(logs / 'string-to-number-mutants.json').write_text(json.dumps(summary, indent=2) + '\n')
