#!/usr/bin/env python3
"""Behavioral mutants; retain only kills at their named contract assertion."""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import fcntl
import json
import subprocess
from leaf_sequences_run import ROOT, LOGS, env

source = ROOT / 'crates/romacs-vm/src/prims/leaf_sequences.rs'
mutants = [
    ('mapconcat_drop_separator', 'parts.push(state[2]);', '/* omitted separator */', 'mapconcat_callback', 'mapconcat_mapping'),
    ('sort_ignore_reverse', 'if values.len() < 64 && reverse != Value::NIL { values.reverse(); }', '/* omitted pre-key reverse */', 'sort_stable_reverse', 'sort_stable_keywords'),
    ('sort_ignore_key', 'if state[2] == Value::NIL || state[2] == cx.intern("identity") || i == n {', 'if true {', 'sort_stable_reverse', 'sort_stable_keywords'),
    ('sort_skip_run_detection', 'if n < 64 {', 'if false {', 'sort_small_gnu_callbacks', 'sort_small_run'),
    ('sort_binary_search_left_bias', 'else { state[11] = number(mid + 1); }', 'else { state[12] = number(mid); }', 'sort_small_gnu_callbacks', 'sort_small_run'),
    ('sort_ignore_inplace', 'if state[4] != Value::NIL {', 'if false {', 'sort_legacy_identity', 'sort_stable_keywords'),
]
results = []
with open(str(_PortablePath(_portable_tempfile.gettempdir()) / 'leaf-build.lock'), 'w') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    original = source.read_text()
    try:
        for name, old, new, assertion, selection in mutants:
            assert original.count(old) == 1, (name, original.count(old))
            source.write_text(original.replace(old, new))
            command = ['cargo', 'test', '-p', 'romacs-vm', '--test', 'leaf_sequences', selection, '--', '--nocapture']
            result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
            output = result.stdout + result.stderr
            (LOGS / f'mutant-{name}.log').write_text(output)
            killed = result.returncode != 0 and 'assertion `left == right` failed: ' + assertion in output and 'error[E' not in output
            results.append(dict(mutant=name, assertion=assertion, status='KILLED' if killed else 'INVALID_OR_SURVIVED', exit=result.returncode))
            assert killed, results[-1]
    finally:
        source.write_text(original)
(LOGS / 'mutants.json').write_text(json.dumps(results, indent=2) + '\n')
print(json.dumps(results, indent=2))
