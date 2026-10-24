#!/usr/bin/env python3
"""Require the named assertion, not any nonzero process exit, to kill a mutant."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


MUTANTS = [
    ('red-eject', 'test_red_ejects_and_regates', 'RED_EJECT: offending tip must not survive',
     '        remaining.remove(culprit)\n        ejected.append(culprit)',
     '        return candidate, ejected'),
    ('red-regate', 'test_red_ejects_and_regates', 'RED_REGATE: remainder must receive its own gate',
     '        if gate(candidate):', '        if ejected or gate(candidate):'),
    ('tree-equality', 'test_replay_divergence_refuses', 'TREE_EQUALITY: unequal replay must refuse verdict transfer',
     '    if expected != observed:', '    if False:'),
    ('fail-closed', 'test_missing_dependency_refuses', 'FAIL_CLOSED: absent fail-closed verification must refuse before gating',
     '    if missing:', '    if False:'),
    ('conflict-eject', 'test_conflict_is_ejected_without_resolution', 'CONFLICT_EJECT: never resolve queue conflicts',
     '            remaining.remove(conflict.tip)', '            return tuple(remaining), ejected'),
    ('kill-cleanup', 'test_kill_cleans_private_ref_without_touching_master', 'KILL_CLEANUP: SIGKILL must leave no orphaned queue ref',
     '    shutil.rmtree(root)', '    pass'),
    ('record-once', 'test_record_once_after_equality', 'RECORD_ONCE: prepare final batch exactly once',
     '    assert_replay_tree(gated, replayed)\n    prepare()',
     '    assert_replay_tree(gated, replayed)\n    prepare()\n    prepare()'),
    ('record-after-equality', 'test_record_once_after_equality', 'RECORD_AFTER_EQUALITY: never prepare a divergent replay',
     '    assert_replay_tree(gated, replayed)\n    prepare()',
     '    prepare()\n    assert_replay_tree(gated, replayed)'),
    ('one-gate', 'test_single_green_batch_one_gate', 'ONE_GATE: green batch needs exactly one gate',
     '        if gate(candidate):', '        if gate(candidate) and gate(candidate):'),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    scripts = Path(__file__).resolve().parent
    original = (scripts / 'lanes_queue.py').read_text()
    rows = []
    for name, test, assertion, old, new in MUTANTS:
        if original.count(old) != 1:
            raise RuntimeError(f'{name}: mutation did not apply uniquely')
        with tempfile.TemporaryDirectory(prefix='queue-mutant-', dir=os.environ.get('TMPDIR')) as directory:
            root = Path(directory)
            for filename in ('lanes', 'lanes_queue_test.py'):
                shutil.copy2(scripts / filename, root / filename)
            (root / 'lanes_queue.py').write_text(original.replace(old, new, 1))
            result = subprocess.run([sys.executable, '-B', str(root / 'lanes_queue_test.py'),
                                     'QueueContracts.' + test], capture_output=True, text=True, timeout=40)
            log = result.stdout + result.stderr
            (args.output / (name + '.log')).write_text(log)
            killed = (result.returncode == 1 and f'FAIL: {test} ' in log
                      and 'AssertionError:' in log and assertion in log
                      and 'FAILED (failures=1)' in log and 'ERROR:' not in log)
            rows.append(dict(mutant=name, test=test, assertion=assertion, rc=result.returncode,
                             verdict='KILLED_AT_TARGET' if killed else 'NOT_A_TARGET_KILL'))
            print(name + ': ' + rows[-1]['verdict'])
    (args.output / 'results.json').write_text(json.dumps(rows, indent=2) + '\n')
    return 0 if all(row['verdict'] == 'KILLED_AT_TARGET' for row in rows) else 1


if __name__ == '__main__':
    sys.exit(main())
