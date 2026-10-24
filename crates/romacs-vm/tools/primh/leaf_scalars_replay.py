#!/usr/bin/env python3
"""Offline replay with a FRESH candidate process per oracle case.

The shared primh CLI's --eval-stream retains current-buffer state across rows;
point-marker's bytebuffer case otherwise contaminates its following cases.
Rust primh tests already create a fresh VM per row. Do not change world lifetime
or add pins to conceal that discrepancy.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
module = importlib.util.spec_from_file_location('primh', HERE / 'primh.py')
primh = importlib.util.module_from_spec(module)
module.loader.exec_module(primh)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--vm2', required=True)
    parser.add_argument('--json', required=True)
    args = parser.parse_args()
    totals = {}
    errors = []
    for path in sorted((HERE / 'leaf-scalar-specs').glob('*.json')):
        spec = primh.load_spec(path)
        evidence = primh.evidence(spec)
        actuals = []
        for row in evidence['cases']:
            p = subprocess.run([args.vm2, '--gc-floor', '1', '--eval-stream'], input=json.dumps({'source': row['source']}) + '\n', text=True, capture_output=True, check=True, timeout=30)
            lines = p.stdout.splitlines()
            assert len(lines) == 1, (row['id'], lines, p.stderr)
            out = json.loads(lines[0])
            result = out.get('result') or {}
            actuals.append(result['value'] if result.get('type') == 'string' else f"VM-ERROR {out.get('error')}")
        counts, bad = primh.ratchet(evidence, actuals)
        totals[spec['name']] = counts
        errors.extend(bad)
        print(spec['name'], json.dumps(counts, sort_keys=True), flush=True)
    Path(args.json).write_text(json.dumps(totals, indent=2) + '\n')
    assert not errors, '\n'.join(errors)

if __name__ == '__main__':
    main()
