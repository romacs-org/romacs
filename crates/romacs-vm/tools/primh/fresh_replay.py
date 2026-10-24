#!/usr/bin/env python3
"""Offline primh replay with a new VM process for every row (including pure rows)."""
import argparse
import hashlib
from pathlib import Path
import json
import subprocess
import primh


def preserve_semantic_pins(pins, inspected):
    preserved = {key: kind for key, kind in pins.items() if kind != 'PRINTER'}
    conflicts = set(preserved) & set(inspected)
    if conflicts:
        raise RuntimeError(f'semantic pins changed to printer gaps: {sorted(conflicts)}')
    return preserved | {key: 'PRINTER' for key in inspected}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('spec')
    ap.add_argument('--vm2', required=True)
    ap.add_argument('--json', required=True)
    ap.add_argument('--review-printer', action='store_true')
    args = ap.parse_args()
    spec = primh.load_spec(args.spec)
    binary = Path(args.vm2).resolve()
    binary_sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()
    ev = primh.evidence(spec)
    actuals = []
    inspected = {}
    for row in ev['cases']:
        p = subprocess.run([args.vm2, '--gc-floor', '1', '--eval-stream'],
                           input=json.dumps({'source': row['source']}) + '\n',
                           capture_output=True, text=True, check=True, timeout=30)
        replies = p.stdout.splitlines()
        if len(replies) != 1:
            raise RuntimeError(f"{row['id']}: expected one reply, got {len(replies)}")
        out = json.loads(replies[0])
        result = out.get('result') or {}
        actual = result['value'] if result.get('type') == 'string' else f"VM-ERROR {out.get('error')}"
        actuals.append(actual)
        if primh.verdict(row, actual) == 'PRINTER':
            inspected[row['id']] = {'expected': row['expected'], 'actual': actual,
                                     'source': row['source']}
    if args.review_printer:
        for item in inspected.values():
            if 'circular list printing requires print-circle' not in item['actual']:
                raise RuntimeError(f'not an established circular printer gap: {item}')
        ev['pinned'] = preserve_semantic_pins(ev.get('pinned', {}), inspected)
        primh.atomic_json(primh.EVID / f"{spec['name']}.json", ev)
    if hashlib.sha256(binary.read_bytes()).hexdigest() != binary_sha256:
        raise RuntimeError('VM binary changed during replay')
    counts, bad = primh.ratchet(ev, actuals)
    primh.atomic_json(args.json, {'primitive': spec['name'], 'fresh_process_per_case': True,
                                'vm_binary': str(binary), 'vm_binary_sha256': binary_sha256,
                                'count': len(actuals), 'counts': counts,
                                'bad': bad, 'reviewed_printer': inspected})
    print(json.dumps({'count': len(actuals), 'counts': counts, 'bad': bad}))
    return bool(bad)


if __name__ == '__main__':
    raise SystemExit(main())
