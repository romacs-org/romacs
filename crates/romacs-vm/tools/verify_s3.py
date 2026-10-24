#!/usr/bin/env python3
"""Verify the combined VM2 evidence; stale binaries or partial runs fail closed."""
import hashlib
import json
from pathlib import Path
from bench import ROOT, SOURCE, load


def main():
    read = lambda n: json.loads((ROOT / 'evidence' / (n + '.json')).read_text())
    binary_hash = hashlib.sha256((ROOT / 'target/release/romacs-vm').read_bytes()).hexdigest()
    semantic = []
    for name, count in [('s3-lang', 58), ('s3-pending', 14), ('s3-known-answers', 60)]:
        report = read(name)
        assert len(report['cases']) == count
        assert all(c['passed'] and c['ir_ok'] for c in report['cases'])
        semantic.append(count)
    cases = {name: (n, body) for name, n, body in load(SOURCE, 's3_original_cases').CASES if name != 'buf-insert'}
    slopes, control, evaluation = [read(n) for n in ['s3-slopes', 's3-slopes-no-collection', 's3-eval']]
    for report, count in [(slopes, 60), (control, 60), (evaluation, 180)]:
        assert report['complete'] and report['binary_sha256'] == binary_hash
        assert len(report['samples']) == count
        assert {c['name'] for c in report['cases']} == set(cases)
        for sample in report['samples']:
            assert sample['count'] > 0 and sample['enabled'] > 0
            assert sample['enabled'] == sample['running']
            if 'checked_responses' in sample:
                assert sample['checked_responses'] == sample['requests']
                assert [json.loads(s) for s in sample['stdout'].splitlines()] == [sample['expected']] * sample['requests']
            else:
                assert sample['stdout'] == sample['expected']
    for report in [slopes, control]:
        assert report['resolution_instructions'] == 36
        for row in report['cases']:
            assert (row['n'], row['body']) == cases[row['name']]
            for engine in ['vm2', 'gnu']:
                assert row['medians'][engine + '/2'] - row['medians'][engine + '/1'] > 36
            assert row['ratio'] < 1
    assert slopes['gc_floor'] is None and control['gc_floor'] == 1_000_000_000
    for row in evaluation['cases']:
        assert row['original_body'] == cases[row['name']][1]
        assert row['source'] == row['original_body'].replace('N', str(row['n']))
    for name in ['s3-gc-dev', 's3-gc-release']:
        gate = read(name)
        assert gate['passed'] and gate['exit'] == 0
        assert 'allocations=600115 reclaimed=600110 live=5 collections=23' in gate['stderr']
    red = read('s3-gc-red')
    assert not red['passed'] and 'exact cons/string/vector allocation accounting' in red['stderr']
    assert 'left: 3' in red['stderr'] and 'right: 10' in red['stderr']
    assert read('s3-threading')['passed'] and read('s3-stream')['passed']
    cost = read('s3-cost-control')
    assert cost['enabled']['collections_during_run'] == 4
    assert cost['control']['collections_during_run'] == 0
    assert cost['enabled']['allocations'] == cost['control']['allocations'] == 200_000
    memory = read('s3-memory')
    assert memory['complete'] and len(memory['cases']) == 5
    assert all(c['peak_rss_ratio'] < 5 for c in memory['cases'])
    for row in memory['stress']:
        assert row['gc']['stats']['allocations'] == row['gc']['stats']['reclaimed'] == row['n']
    receipt = dict(semantic_cases=semantic, samples=[60, 60, 180],
        checked_eval_replies=sum(s['checked_responses'] for s in evaluation['samples']),
        exact_scheduling=True, all_five_below_gnu_bytecode=True, resolution_instructions=36,
        gc_red_then_green=True, peak_rss_ceiling_passed=True, binary_sha256=binary_hash,
        rust_sources_sha256={str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted((ROOT / 'src').glob('*.rs'))})
    (ROOT / 'evidence/s3-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
