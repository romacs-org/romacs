#!/usr/bin/env python3
"""Fail closed on incomplete evidence, then enforce the new implementation bar."""
import argparse
import hashlib
import json
import math
import statistics
import subprocess
import sys
from pathlib import Path

if not __debug__:
    raise RuntimeError('Assertions are part of the fail-closed verification guard; do not use python -O')

p = argparse.ArgumentParser()
p.add_argument('readings', type=Path)
p.add_argument('--ascii-baseline', type=Path)
p.add_argument('--validate-only', action='store_true', help='verify raw evidence only; do NOT evaluate performance acceptance')
a = p.parse_args()
verification = json.loads((a.readings / 'verification.json').read_text())
assert verification['runtime_unchanged'] is True
for name in ['raw', 'results']:
    assert hashlib.sha256((a.readings / f'{name}.jsonl').read_bytes()).hexdigest() == verification[f'{name}_sha256'], 'receipt content mismatch'
workloads = json.loads((Path(__file__).parent / 'workloads.json').read_text())['workloads']
rows = [json.loads(line) for line in (a.readings / 'results.jsonl').read_text().splitlines()]
raw_rows = [json.loads(line) for line in (a.readings / 'raw.jsonl').read_text().splitlines()]
raw = {r['tag']: r for r in raw_rows}
assert len(raw) == len(raw_rows), 'duplicate raw record tags'
expected = {(w['name'], fixture) for w in workloads for fixture in ['mixed', 'ascii']}
assert len(rows) == len(expected) == len({(r['name'], r['fixture']) for r in rows})
assert {(r['name'], r['fixture']) for r in rows} == expected
manifest = json.loads((a.readings / 'manifest.json').read_text())
assert manifest['runtime_sha256'], 'missing runtime fingerprint'
assert manifest['workloads_sha256'] == hashlib.sha256((Path(__file__).parent / 'workloads.json').read_bytes()).hexdigest(), 'workload source changed'
failures = []
for r in rows:
    case = next(w for w in workloads if w['name'] == r['name'])
    assert r['iterations'] == case['iterations'], 'iteration count differs from pinned workload'
    tags = r['accepted_tags']
    assert len(tags) == len(set(tags)) == 12, 'three complete paired slopes required'
    slopes = {'romacs': [], 'gnu': []}
    for rep in range(3):
        pair = {}
        for engine, mult in [('romacs', 1), ('gnu', 1), ('gnu', 2), ('romacs', 2)]:
            prefix = f"{r['fixture']}-{r['name']}-{rep}-{engine}-{mult}-try"
            matches = [raw[tag] for tag in tags if tag.startswith(prefix)]
            assert len(matches) == 1
            sample = matches[0]
            assert sample['rc'] == 0 and sample['count'] > 0, 'invalid count or editor exit'
            assert sample['running'] == sample['enabled'] > 0, 'counter scheduling rejected'
            assert sample['load'], 'missing load snapshot'
            answer = [s for s in sample['output'].splitlines() if s.startswith('SWEEP=')]
            assert len(answer) == 1
            if engine == 'romacs':
                image = [s for s in sample['output'].splitlines() if s.startswith('IMAGE=')]
                assert len(image) == 1 and manifest['image'] in image[0], 'shipping image proof missing'
                assert '(dumped-with-pdumper . t)' in image[0]
            pair[engine, mult] = sample, answer[0]
        for mult in [1, 2]:
            assert pair['romacs', mult][1] == pair['gnu', mult][1], 'cross-engine answer mismatch'
        for engine in slopes:
            delta = pair[engine, 2][0]['count'] - pair[engine, 1][0]['count']
            assert delta > 36
            slopes[engine].append(delta / r['iterations'])
    for engine in slopes:
        assert slopes[engine] == r['slopes'][engine], 'summary not backed by raw counts'
        assert statistics.median(slopes[engine]) == r[engine]
    ratio = r['romacs'] / r['gnu']
    assert math.isfinite(ratio) and ratio == r['ratio']
    target = next(w['target_mixed_ratio'] for w in workloads if w['name'] == r['name'])
    if r['fixture'] == 'mixed' and ratio > target:
        failures.append(f"{r['name']}: {ratio:.3f}x > {target:.1f}x")
if a.validate_only:
    print(f'Validated {len(rows)} rows and {sum(len(r["accepted_tags"]) for r in rows)} accepted exact-scheduling counter records; performance acceptance NOT evaluated')
    raise SystemExit(0)
if a.ascii_baseline:
    assert a.ascii_baseline.resolve() != a.readings.resolve(), 'ASCII baseline must be a separate before acquisition'
    subprocess.run([sys.executable, str(Path(__file__).resolve()), str(a.ascii_baseline), '--validate-only'], check=True)
    before = {r['name']: r for r in map(json.loads, (a.ascii_baseline / 'results.jsonl').read_text().splitlines()) if r['fixture'] == 'ascii'}
    assert set(before) == {w['name'] for w in workloads}
    for r in rows:
        if r['fixture'] != 'ascii': continue
        old = before[r['name']]
        # Disjoint triplets separated by >36 total instructions are evidence
        # of regression; overlapping readings are inconclusive, not a win.
        if (min(r['slopes']['romacs']) - max(old['slopes']['romacs'])) * r['iterations'] > 36:
            failures.append(f"{r['name']}: ASCII regression beyond baseline triplet and resolution")
else:
    print('ASCII no-regression comparison: NOT RUN (supply --ascii-baseline)')
    failures.append('ASCII no-regression evidence missing')
print(f'Validated {len(rows)} rows and {sum(len(r["accepted_tags"]) for r in rows)} accepted exact-scheduling counter records')
print(f'Performance failures: {len(failures)}')
for failure in failures:
    print(f'FAIL {failure}')
raise SystemExit(bool(failures))
