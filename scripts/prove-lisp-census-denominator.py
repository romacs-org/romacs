#!/usr/bin/env python3
"""Focused R218 red replay + isolated in-memory mutants; no Cargo or census."""
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import types
import unittest

ROOT = Path(__file__).resolve().parents[1]
path = ROOT / 'scripts/lisp-census.py'
spec = importlib.util.spec_from_file_location('denominator_tests', ROOT / 'scripts/test_lisp_census_denominator.py')
tests = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tests)


def run(source):
    module = types.ModuleType('census_proof')
    module.__file__ = str(path)
    exec(compile(source, str(path), 'exec'), module.__dict__)
    tests.census = module
    stream = io.StringIO()
    result = unittest.TextTestRunner(stream=stream).run(unittest.defaultTestLoader.loadTestsFromModule(tests))
    return {'passed': result.wasSuccessful(), 'failures': len(result.failures), 'errors': len(result.errors), 'log': stream.getvalue()}


source = path.read_text()
base = '612557064d1804ece86bed1ffcd91ab1e67b008e'
original = subprocess.check_output(['git', 'show', base + ':scripts/lisp-census.py'], cwd=ROOT, text=True)
red = run(original)
assert red['failures'] == 1 and red['errors'] == 0, red
green = run(source)
assert green['passed'], green
mutations = {
    'gnu-file-hashes-fabricated': ('hashlib.sha256(original).hexdigest()', "'0' * 64"),
    'generated-files-counted-as-bundled': ("tracked = {p for p in tracked if p.endswith('.el')}", "tracked = {p for p in tracked if p.endswith('.el')} | {str(p.relative_to(ROOT)) for p in (ROOT / 'lisp').rglob('*.el')}"),
    'bundle-counted-as-parity': ('len(successful & shared)', 'len(successful)'),
    'gnu-target-loses-missing': ("'gnu_target_total': len(entries)", "'gnu_target_total': len(shared)"),
    'missing-paths-erased': ("'missing_paths': missing", "'missing_paths': []"),
    'moving-gnu-HEAD-as-oracle': ("['git', 'archive', commit, 'lisp']", "['git', 'archive', 'HEAD', 'lisp']"),
    'dirty-gnu-worktree-as-oracle': ('original = tree.extractfile(member).read()', 'original = (GNU_SOURCE / member.name).read_bytes()'),
    'drift-hidden': ("'status': 'missing' if member.name not in tracked else 'pristine' if local == original else 'modified'", "'status': 'missing' if member.name not in tracked else 'pristine'"),
    'failed-modified-counted-as-loaded': ('len(successful & modified)', 'len(modified)'),
    'failed-bundled-counted-as-loaded': ("len(successful & set(inventory['bundled_paths']))", "len(inventory['bundled_paths'])"),
    'warm-summary-omitted': ('summary.update(parity_summary(rows, inventory if inventory is not None else oracle_inventory()))', "summary.update(parity_summary(rows, inventory if inventory is not None else oracle_inventory()) if mode == 'cold' else {})"),
    'human-figure-wrong-denominator': ("{p['loaded']}/{p['shared_total']} shared", "{p['loaded']}/{p['gnu_target_total']} shared"),
}
results = {}
for name, (old, new) in mutations.items():
    assert source.count(old) == 1, (name, source.count(old))
    results[name] = run(source.replace(old, new))
    assert results[name]['failures'] >= 1 and results[name]['errors'] == 0, (name, results[name])
report = {'base_commit': base, 'red': red, 'green': green, 'mutants': results,
          'scope': 'synthetic git fixture only; no VM measurement, no Cargo builds'}
output = ROOT / 'LISP-CENSUS-DENOMINATOR-PROOF.json'
output.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({'output': str(output), 'red_assertion_failures': red['failures'], 'green': green['passed'], 'mutants_killed': len(results)}))
