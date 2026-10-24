#!/usr/bin/env python3
"""Verify old oracle rows survive fresh recording; replay in fresh candidate worlds."""
import os as _portable_os
from pathlib import Path as _PortablePath
import hashlib
import json
import subprocess
from leaf_sequences_run import ROOT, LOGS, env

binary = _portable_os.environ.get('ROMACS_VM2_BIN', str(next(p for p in _PortablePath(__file__).resolve().parents if (p / 'parity-reference.toml').is_file()) / 'target/debug/romacs-vm'))
before_hash = hashlib.sha256(open(binary, 'rb').read()).hexdigest()
remaining, preservation, counts = [], [], {}
for name in ['mapconcat', 'sort']:
    path = f'crates/romacs-vm/tools/primh/evidence/{name}.json'
    old = json.loads(subprocess.check_output(['git', 'show', f'26ab096a:{path}'], cwd=ROOT))
    fresh = json.loads((ROOT / path).read_text())
    old_rows = {(r['id'], r['family'], r['source']): r for r in old['cases']}
    fresh_rows = {(r['id'], r['family'], r['source']): r for r in fresh['cases']}
    assert len(fresh_rows) == len(fresh['cases'])
    for identity, prior in old_rows.items():
        assert identity in fresh_rows, identity
        current = fresh_rows[identity]
        for field in ['expected', 'gnu_regime_variant']:
            assert prior.get(field) == current.get(field), (identity, field)
    assert not fresh.get('pinned'), 'strict evidence must not bless partials'
    preservation.append(dict(name=name, preserved=len(old_rows), fresh=len(fresh_rows), answers_unchanged=True))
    passed = 0
    for case in fresh['cases']:
        request = json.dumps({'source': case['source']}) + '\n'
        result = subprocess.run([binary, '--gc-floor', '1', '--eval-stream'], input=request,
                                capture_output=True, text=True, check=True, env=env)
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        assert len(replies) == 1
        actual = replies[0]['result']['value']
        if actual not in [case['expected'], case.get('gnu_regime_variant')]:
            remaining.append(dict(id=case['id'], source=case['source'], expected=case['expected'],
                                  alternate=case.get('gnu_regime_variant'), actual=actual))
        else:
            passed += 1
    counts[name] = dict(cases=len(fresh_rows), passed=passed, divergent=len(fresh_rows)-passed)
assert before_hash == hashlib.sha256(open(binary, 'rb').read()).hexdigest(), 'candidate changed during replay'
receipt = dict(binary=binary, sha256=before_hash, prior_commit='26ab096a', preservation=preservation,
               counts=counts, remaining=remaining)
(LOGS / 'small-sort-replay.json').write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + '\n')
(LOGS / 'divergences.json').write_text(json.dumps(remaining, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(receipt, ensure_ascii=False, indent=2))
