#!/usr/bin/env python3
"""Focused compiling mutants; restore every edited byte even on failure.
Run from an owned checkout with the prepared project environment.
Raw command output and a machine-readable receipt go to --out (scratch).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--out', required=True, type=Path)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
root = Path(__file__).resolve().parents[3]
source = root / 'crates/romacs-vm/src/regexp.rs'
frontier = root / 'crates/romacs-vm/tests/custom_bootstrap.rs'
mutants = [
    ('replacement-bytes', source, 'bytes.extend_from_slice(replacement.bytes());', 'bytes.extend_from_slice(&[]);', 'replace_string_bootstrap', 'REPLACE_STRING'),
    ('character-offset', source, 'source.bytes[..source.byte(a)]', 'source.bytes[..source.byte(a.saturating_sub(usize::from(!source.bytes.is_ascii())))]', 'replace_string_bootstrap', 'REPLACE_STRING'),
    ('subexpression', source, '            n as usize\n        };', '            0usize\n        };', 'replace_string_bootstrap', 'REPLACE_STRING'),
    ('preserve-registers', source, '        Ok(self.make_lisp_string(bytes, input.is_multibyte() || replacement.is_multibyte()))', '        self.regexp.groups.clear();\n        Ok(self.make_lisp_string(bytes, input.is_multibyte() || replacement.is_multibyte()))', 'replace_string_bootstrap', 'REPLACE_STRING'),
    ('newtext-type', source, 'ok_or(Fault::Wrong("stringp", newtext))?', 'ok_or(Fault::Wrong("listp", newtext))?', 'replace_rejects_invalid', 'REPLACE_VALIDATION'),
    ('register-refusal', source, 'if n < 0 || n as usize >= self.regexp.groups.len() {\n                return Err(Fault::Error("VM2 replace-match unsupported: unrepresented subexpression register".into()));\n            }', 'if n < 0 || n as usize >= self.regexp.groups.len() { return Ok(Value::NIL); }', 'replace_unrepresented', 'REPLACE_REGISTER_REFUSAL'),
    ('bignum-refusal', source, 'if matches!(self.object_if(subexp), Some(Object::Bignum(_))) {\n                return Err(Fault::Error("VM2 replace-match unsupported: unrepresented subexpression register".into()));\n            }', 'if matches!(self.object_if(subexp), Some(Object::Bignum(_))) { return Ok(Value::NIL); }', 'replace_unrepresented', 'REPLACE_REGISTER_REFUSAL'),
    ('signed-refusal', source, 'return Err(refuse("signed or inverted match registers"));', 'return Ok(Value::NIL);', 'replace_signed', 'REPLACE_SIGNED_REFUSAL'),
    ('case-refusal', source, 'if fixedcase == Value::NIL && !replacement.bytes().is_empty()', 'if false && fixedcase == Value::NIL && !replacement.bytes().is_empty()', 'replace_unsupported', 'REPLACE_REFUSAL: case inference'),
    ('backslash-refusal', source, "if literal == Value::NIL && replacement.bytes().contains(&b'\\\\')", "if false && literal == Value::NIL && replacement.bytes().contains(&b'\\\\')", 'replace_unsupported', 'REPLACE_REFUSAL: replacement backslash'),
    ('properties-refusal', source, 'if matches!(self.object_if(value), Some(Object::String(s) | Object::RawString(s)) if !s.properties.is_empty())', 'if false && matches!(self.object_if(value), Some(Object::String(s) | Object::RawString(s)) if !s.properties.is_empty())', 'replace_unsupported', 'REPLACE_REFUSAL: text properties'),
    ('encoding-refusal', source, 'return Err(refuse("mixed non-ASCII string encodings"));', 'return Ok(Value::NIL);', 'replace_unsupported', 'REPLACE_REFUSAL: mixed non-ASCII'),
    ('buffer-refusal', source, 'if string == Value::NIL { return Err(refuse("buffer replacement")); }', 'if string == Value::NIL { return Ok(Value::NIL); }', 'replace_unsupported', 'REPLACE_BUFFER_REFUSAL'),
    ('frontier-pin', frontier, 'line 494 form 50:', 'line 495 form 50:', '', 'CUSTOM_FILES_FRONTIER_MOVED'),
]
rows = []
for name, path, old, new, select, marker in mutants:
    original = path.read_bytes()
    text = original.decode()
    if text.count(old) != 1:
        raise RuntimeError(f'{name}: expected one mutation site, got {text.count(old)}')
    command = ['cargo', 'test', '-p', 'romacs-vm', '--test', 'custom_bootstrap' if path == frontier else 'replace_match']
    if select:
        command.append(select)
    command += ['--', '--nocapture']
    try:
        path.write_text(text.replace(old, new))
        run = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
        log = a.out / f'{name}.log'
        log.write_bytes(run.stdout)
        output = run.stdout.decode(errors='replace')
        assertion = 'panicked at' if path == frontier else 'assertion `left == right` failed'
        killed = run.returncode == 101 and 'test result: FAILED' in output and assertion in output and marker in output
        rows.append(dict(name=name, command=command, exit=run.returncode, marker=marker, killed_at_assertion=killed, original_sha256=hashlib.sha256(original).hexdigest(), log=str(log), log_sha256=hashlib.sha256(run.stdout).hexdigest()))
    finally:
        path.write_bytes(original)
    rows[-1]['restored'] = path.read_bytes() == original
    (a.out / 'receipt.json').write_text(json.dumps(rows, indent=2) + '\n')
    print(f'{name}: exit={run.returncode} assertion_kill={killed} restored={rows[-1]["restored"]}', flush=True)
    if not killed:
        raise RuntimeError(f'{name}: no qualifying assertion kill; see {log}')
if not all(r['killed_at_assertion'] and r['restored'] for r in rows):
    raise RuntimeError('unqualified mutation')
print(f'{len(rows)}/{len(mutants)} compiling mutants killed at named assertions')
