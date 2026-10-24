#!/usr/bin/env python3
"""Capture exact unpinned oracle mismatches and isolated pin probes."""
import os as _portable_os
from pathlib import Path as _PortablePath
import json
import resource
import subprocess
import tempfile
from leaf_sequences_run import ROOT, LOGS, env

binary = _portable_os.environ.get('ROMACS_VM2_BIN', str(next(p for p in _PortablePath(__file__).resolve().parents if (p / 'parity-reference.toml').is_file()) / 'target/debug/romacs-vm'))
rows = []
for name in ['mapconcat', 'sort']:
    evidence = json.loads((ROOT / f'crates/romacs-vm/tools/primh/evidence/{name}.json').read_text())
    cases = evidence['cases']
    request = ''.join(json.dumps({'source': case['source']}) + '\n' for case in cases)
    result = subprocess.run([binary, '--gc-floor', '1', '--eval-stream'], input=request, capture_output=True, text=True, check=True, env=env)
    replies = [json.loads(line) for line in result.stdout.splitlines()]
    assert len(replies) == len(cases)
    for case, reply in zip(cases, replies):
        actual = reply['result']['value']
        if actual not in [case['expected'], case.get('gnu_regime_variant')]:
            rows.append(dict(id=case['id'], source=case['source'], expected=case['expected'], alternate=case.get('gnu_regime_variant'), actual=actual))
(LOGS / 'divergences.json').write_text(json.dumps(rows, ensure_ascii=False, indent=2) + '\n')
probes = [
    '(list emacs-version emacs-repository-version (help-function-arglist (quote sort)) (documentation (quote sort)))',
    '(sort (quote ((2 . a) (1 . b) (2 . c))) :key (function car) :lessp (function <) :reverse t)',
    '(let ((x (list "a" "b"))) (mapconcat (lambda (y) (setcdr x nil) y) x "-"))',
]
observations = []
with tempfile.TemporaryDirectory(dir=env['TMPDIR'], prefix='leaf-sequences-gnu-') as private:
    gnu_env = dict(env, HOME=private)
    for expression in probes:
        def no_core():
            resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        result = subprocess.run([env['ROMACS_GNU_ORACLE'], '-Q', '--batch', '--eval', f'(prin1 {expression})'], env=gnu_env, cwd=private, capture_output=True, text=True, preexec_fn=no_core, timeout=30)
        observations.append(dict(source=expression, exit=result.returncode, stdout=result.stdout, stderr=result.stderr))
(LOGS / 'pin-probes.json').write_text(json.dumps(observations, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(rows, ensure_ascii=False, indent=2))
print('GNU isolated probe exits:', [row['exit'] for row in observations])
