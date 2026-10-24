#!/usr/bin/env python3
"""Verify preserved GNU evidence and explicitly ledgered boundary pins."""
import argparse
from collections import defaultdict, deque
import hashlib
import json
from pathlib import Path
import subprocess
import primh


REVIEWED = {
    'read-from-string': {
        'read-from-string/0163': ('VALUE', 'VM2-READ2'),
        'read-from-string/0164': ('VALUE', 'VM2-READ2'),
        'read-from-string/0165': ('VALUE', 'VM2-READ2'),
        'read-from-string/0166': ('VALUE', 'VM2-READ3'),
        'read-from-string/0167': ('VALUE', 'VM2-READ3'),
        'read-from-string/0168': ('SIGNAL-VS-VALUE', 'VM2-READ4'),
        'read-from-string/0169': ('SIGNAL-VS-VALUE', 'VM2-READ4'),
        'read-from-string/0170': ('DATA', 'VM2-READ5'),
        'read-from-string/0171': ('SIGNAL-VS-VALUE', 'VM2-READ1'),
        'read-from-string/0174': ('VALUE', 'VM2-READ3'),
    },
    'string-to-number': {},
    'set-match-data': {
        'set-match-data/0507': ('SIGNAL-VS-VALUE', 'VM2-MD3'),
    },
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--old-dir', type=Path, required=True)
    ap.add_argument('--vm2', type=Path, required=True)
    ap.add_argument('--json', required=True)
    args = ap.parse_args()
    crate = primh.HERE.parent.parent
    inputs = sorted(set(crate.glob('src/**/*.rs')) | set(crate.glob('tests/*.rs')) |
                    set(primh.HERE.glob('*.py')) | set(primh.HERE.glob('specs/*.json')) |
                    {crate / 'Cargo.toml', crate / 'build.rs'})
    proof = {'vm_binary_sha256': hashlib.sha256(args.vm2.read_bytes()).hexdigest(),
             'inputs_sha256': {str(path.relative_to(crate)): hashlib.sha256(path.read_bytes()).hexdigest()
                               for path in inputs}, 'primitives': {}}
    for name, reviewed in REVIEWED.items():
        old_path = args.old_dir / f'{name}-review-old.json'
        old_bytes = old_path.read_bytes()
        old = json.loads(old_bytes)
        ev = primh.evidence(primh.load_spec(primh.HERE / 'specs' / f'{name}.json'))
        by_form = defaultdict(deque)
        for row in ev['cases']:
            by_form[(row['family'], row['source'])].append(row)
        pins, mappings = {}, []
        for row in old['cases']:
            current = by_form[(row['family'], row['source'])].popleft()
            for field in ('expected', 'gnu_regime_variant'):
                assert current.get(field) == row.get(field), (name, row['id'], field)
            if row['id'] in old.get('pinned', {}):
                pins[current['id']] = old['pinned'][row['id']]
            mappings.append({'old_id': row['id'], 'current_id': current['id']})
        new_ids = {row['id'] for rows in by_form.values() for row in rows}
        assert set(reviewed) <= new_ids
        probes = []
        for row in ev['cases']:
            if row['id'] not in new_ids:
                continue
            p = subprocess.run([str(args.vm2), '--gc-floor', '1', '--eval-stream'],
                               input=json.dumps({'source': row['source']}) + '\n',
                               text=True, capture_output=True, check=True, timeout=30)
            reply = json.loads(p.stdout)
            result = reply.get('result') or {}
            actual = result['value'] if result.get('type') == 'string' else f"VM-ERROR {reply.get('error')}"
            kind = primh.verdict(row, actual)
            if row['id'] in reviewed:
                expected_kind, ledger = reviewed[row['id']]
                assert kind == expected_kind, (row['id'], kind, expected_kind)
                pins[row['id']] = kind
            else:
                assert kind == 'PASS', (row['id'], kind)
                ledger = None
            probes.append(dict(row, actual=actual, classification=kind, divergence=ledger))
        ev['pinned'] = pins
        primh.atomic_json(primh.EVID / f'{name}.json', ev)
        proof['primitives'][name] = {
            'old_evidence_sha256': hashlib.sha256(old_bytes).hexdigest(),
            'old_case_count': len(old['cases']), 'current_case_count': len(ev['cases']),
            'preserved_form_and_gnu_answer_count': len(mappings),
            'old_pin_count': len(old.get('pinned', {})),
            'restored_old_pin_count': len(pins) - len(reviewed),
            'mappings': mappings, 'new_probes': probes,
        }
    primh.atomic_json(args.json, proof)
    print(json.dumps({name: {key: val for key, val in data.items() if key not in ('mappings', 'new_probes')}
                      for name, data in proof['primitives'].items()}, indent=2))


if __name__ == '__main__':
    main()
