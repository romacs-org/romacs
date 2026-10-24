#!/usr/bin/env python3
"""Later qualification of the exact production diagnostic transport (not TDD).
Extract its std-only implementation and tests; compile unchanged and zero-mutant.
Does not alter shared source, root manifests or the atlas implementation.
"""
from pathlib import Path
import argparse
import json
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=False)
crate = Path(__file__).resolve().parents[1]
source = (crate / 'src/paint_host.rs').read_text()
source = source[source.index('/// A sample from the live window atlas'):source.index('pub struct PaintHost {')]
needle = '*self.latest.lock().unwrap() = Some(diagnostic);'
assert source.count(needle) == 1
mutant = source.replace(needle, '*self.latest.lock().unwrap() = Some(PaintDiagnostic { unresolved_face_text_total: 0, ..diagnostic });')
results = {}
for name, text in [('baseline', source), ('zero-mutant', mutant)]:
    rust = args.output / (name + '.rs')
    binary = args.output / name
    rust.write_text(text)
    build = subprocess.run(['rustc', '--edition=2024', '--test', '--crate-name', 'diagnostic_probe', str(rust), '-o', str(binary)], capture_output=True, text=True)
    (args.output / (name + '-build.log')).write_text(build.stdout + build.stderr)
    if build.returncode:
        raise RuntimeError('compilation is not a behavioral RED: ' + build.stderr)
    result = subprocess.run([str(binary.resolve()), '--nocapture'], capture_output=True, text=True)
    (args.output / (name + '.log')).write_text(result.stdout + result.stderr)
    results[name] = result.returncode
assert results['baseline'] == 0
assert results['zero-mutant'] == 101
(args.output / 'summary.json').write_text(json.dumps({'qualification': 'later transport zero-substitution mutant, not atlas-path or red-first proof', 'exit_codes': results}, indent=2) + '\n')
print(json.dumps(results))
