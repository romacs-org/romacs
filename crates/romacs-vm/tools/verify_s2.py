#!/usr/bin/env python3
"""Check completeness, guards, original workload bodies and binary provenance."""
import hashlib
import json
from pathlib import Path
from bench import ROOT, SOURCE, load


def main():
    read = lambda name: json.loads((ROOT / 'evidence' / name).read_text())
    lang, pending, extra = (read(n) for n in ('lang-known-answers.json', 'lang-pending.json', 's2-known-answers.json'))
    for report, count in ((lang, 58), (pending, 14), (extra, 60)):
        assert len(report['cases']) == count
        assert all(row['passed'] and row['ir_ok'] for row in report['cases'])
    slopes, evaluation = read('s2-slopes.json'), read('s2-eval.json')
    cases = {name: (n, body) for name, n, body in load(SOURCE, 'original_cases').CASES if name != 'buf-insert'}
    assert slopes['complete'] and evaluation['complete']
    binary_hash = hashlib.sha256((ROOT / 'target/release/romacs-vm').read_bytes()).hexdigest()
    assert slopes['binary_sha256'] == evaluation['binary_sha256'] == binary_hash
    assert {r['name'] for r in slopes['cases']} == set(cases)
    assert {r['name'] for r in evaluation['cases']} == set(cases)
    for row in slopes['cases']:
        assert (row['n'], row['body']) == cases[row['name']]
    for row in evaluation['cases']:
        assert row['original_body'] == cases[row['name']][1]
        assert row['source'] == row['original_body'].replace('N', str(row['n']))
    for report, count in ((slopes, 60), (evaluation, 180)):
        assert len(report['samples']) == count
        for sample in report['samples']:
            assert sample['count'] > 0 and sample['enabled'] > 0 and sample['enabled'] == sample['running']
            if 'checked_responses' in sample:
                assert sample['checked_responses'] == sample['requests']
                assert [json.loads(line) for line in sample['stdout'].splitlines()] == [sample['expected']] * sample['requests']
            else:
                assert sample['stdout'] == sample['expected']
    assert read('s2-threading.json')['passed'] and read('s2-stream.json')['passed']
    out = dict(semantic_rows=[58, 14, 60], baseline_samples=len(slopes['samples']),
               external_eval_samples=len(evaluation['samples']),
               checked_eval_replies=sum(s['checked_responses'] for s in evaluation['samples']),
               rejected_measurements=len(slopes['rejections']) + len(evaluation['rejections']),
               original_bodies_verified=True, positive_exactly_scheduled=True,
               release_binary_sha256=hashlib.sha256((ROOT / 'target/release/romacs-vm').read_bytes()).hexdigest(),
               rust_sources_sha256={str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((ROOT / 'src').glob('*.rs'))},
               gc_integration_passed=read('s2-gc-integration.json')['passed'],
               scope='isolated VM2; NDJSON already-read form control boundary, not production deployment or an Elisp source reader')
    (ROOT / 'evidence/s2-verification.json').write_text(json.dumps(out, indent=2) + '\n')
    print(json.dumps(out, indent=2))

if __name__ == '__main__':
    main()
