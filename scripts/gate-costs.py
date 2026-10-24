#!/usr/bin/env python3
"""Rank a completed full gate's test costs; compare exact test identities.

Reads existing evidence only. Does not run, filter, retry or modify any test.
Per-test seconds are overlapping elapsed durations, not CPU time. Suite seconds
are nextest execution wall time, excluding gate preparation and compilation.
"""
import argparse
import collections
import json
import math
from pathlib import Path
import re
import sys


def read_run(root):
    root = Path(root)
    receipt = json.loads((root / 'receipt.json').read_text())
    if not (receipt.get('complete') is True and receipt.get('verdict') == 'GREEN'
            and receipt.get('mode') == 'full' and receipt.get('repeat') == 1):
        raise ValueError('requires a complete GREEN, single-run full gate')
    names, rows, durations = {}, [], {}
    for suite in receipt['suites']:
        inventory = json.loads((root / f'{suite}-tests.json').read_text())['rust-suites']
        selected, ignored = set(), set()
        for binary in inventory.values():
            for name, case in binary['testcases'].items():
                identity = f"{binary['package-name']}::{binary['binary-name']}${name}"
                if case['ignored']:
                    ignored.add(identity)
                elif case['filter-match']['status'] == 'matches':
                    selected.add(identity)
        actual = collections.Counter()
        for line in (root / f'{suite}.jsonl').read_text().splitlines():
            if not line.startswith('{'):
                continue
            event = json.loads(line)
            if event.get('type') != 'test' or event.get('event') != 'ok':
                continue
            name, seconds = event['name'], event['exec_time']
            if not isinstance(seconds, (int, float)) or not math.isfinite(seconds) or seconds < 0:
                raise ValueError(f'invalid duration: {name}')
            actual[name] += 1
            rows.append({'suite': suite, 'name': name, 'seconds': seconds})
        if set(actual) != selected or any(n != 1 for n in actual.values()):
            raise ValueError(f'{suite}: results do not match full selected test identities')
        if len(selected) != receipt['selected'][suite]:
            raise ValueError(f'{suite}: receipt selection differs from inventory')
        names[suite] = {'selected': sorted(selected), 'inventory_ignored': sorted(ignored)}
        matches = re.findall(r'Summary \[\s*([0-9.]+)s\]', (root / f'{suite}.log').read_text())
        if len(matches) != 1:
            raise ValueError(f'{suite}: expected exactly one nextest summary')
        durations[suite] = float(matches[0])
    if set(receipt['passed']) != {row['name'] for row in rows}:
        raise ValueError('receipt pass identities differ from native output')
    return {'run': str(root.resolve()), 'tree': receipt['tree'],
            **{key: receipt[key] for key in ('gate_sha', 'env_fp', 'runner_sha',
                                            'oracle_sha', 'oracle_source', 'tool_hashes')},
            'inventory': names, 'suite_seconds': durations,
            'costs': sorted(rows, key=lambda row: (-row['seconds'], row['name']))}


def compare(before, after):
    if before['inventory'] != after['inventory']:
        raise ValueError('full selected/ignored test-name lists differ')
    for key in ('gate_sha', 'env_fp', 'runner_sha', 'oracle_sha', 'oracle_source', 'tool_hashes'):
        if before[key] != after[key]:
            raise ValueError(f'{key} differs: not a scheduling-only comparison')
    return {'identical_test_names': True, 'before': before['run'], 'after': after['run'],
            'before_seconds': before['suite_seconds'], 'after_seconds': after['suite_seconds']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run', type=Path)
    parser.add_argument('--compare', type=Path)
    parser.add_argument('--json', action='store_true', help='include complete names and costs')
    parser.add_argument('--top', type=int, default=20)
    args = parser.parse_args()
    try:
        run = read_run(args.run)
        if args.compare:
            result = compare(run, read_run(args.compare))
        elif args.json:
            result = run
        else:
            result = {k: v for k, v in run.items() if k not in ('inventory', 'costs')}
            result['counts'] = {s: {k: len(v) for k, v in n.items()} for s, n in run['inventory'].items()}
            result['costs'] = run['costs'][:args.top]
        print(json.dumps(result, indent=2))
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f'gate-costs: {error}\n')


if __name__ == '__main__':
    main()
