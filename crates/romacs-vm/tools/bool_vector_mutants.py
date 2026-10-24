#!/usr/bin/env python3
"""Run only after parent grants the build slot; baseline must pass first."""
import os as _portable_os
from pathlib import Path as _PortablePath
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / 'evidence/bool-vector'
os.environ['CARGO_TARGET_DIR'] = str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/leaf2-target')
os.environ['CARGO_BUILD_JOBS'] = '4'
os.environ['RUSTC_WRAPPER'] = 'sccache'
def run(test, name):
    p = subprocess.run(['cargo', 'test', '--config', '.cargo/dev.toml', '--profile', 'dev-fast', '-p', 'romacs-vm', '--lib', test, '--', '--nocapture'], cwd=ROOT.parents[1], capture_output=True, text=True)
    output = p.stdout + p.stderr
    (EVIDENCE / (name + '.log')).write_text(output)
    return p.returncode, output

code, output = run('prim::prims::bool_vector::tests', 'baseline')
if code or 'test result: ok.' not in output or '0 passed;' in output:
    raise SystemExit('Baseline did not pass; inspect baseline.log (no mutations applied).')
results = []
for row in json.loads((EVIDENCE / 'mutants.json').read_text()):
    path = ROOT / row['path']
    original = path.read_text()
    if original.count(row['old']) != 1:
        raise SystemExit('Mutation is not unique: ' + row['id'])
    try:
        path.write_text(original.replace(row['old'], row['new']))
        code, output = run(row['test'], row['id'])
        killed = code != 0 and 'assertion `left == right` failed' in output and 'error: could not compile' not in output
        results.append(dict(id=row['id'], killed_at_behavioral_assertion=killed, exit_code=code))
    finally:
        path.write_text(original)
(EVIDENCE / 'mutation-results.json').write_text(json.dumps(results, indent=2) + '\n')
if not all(r['killed_at_behavioral_assertion'] for r in results):
    raise SystemExit('Surviving or invalid mutant: inspect mutation-results.json')
print(json.dumps(results, indent=2))
