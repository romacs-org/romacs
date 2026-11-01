#!/usr/bin/env python3
"""Focused excursion evidence; run mutations in an owned isolated checkout.

No GNU lookup/fallback: --record-gnu uses only the pinned GNU binary.
Mutations are killed only by their named behavioral assertion (not build faults).
"""
import os as _portable_os
from pathlib import Path as _PortablePath
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

CRATE = Path(__file__).resolve().parents[1]
ROOT = CRATE.parents[1]
SOURCE = CRATE / 'src/save_excursion.rs'
TESTS = CRATE / 'tests/save_excursion.rs'
EVIDENCE = CRATE / 'evidence/save-excursion'
GNU = Path(_portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src/src/emacs')))


def record():
    pattern = r'check\(("(?:[^"\\]|\\.)*"), ("(?:[^"\\]|\\.)*"), ("[^"]*")\)'
    cases = [tuple(map(json.loads, row)) for row in re.findall(pattern, TESTS.read_text())]
    gc = re.search(r'vm.eval_source\(("(?:[^"\\]|\\.)*")\)', TESTS.read_text())
    assert gc, 'GC fixture source not found'
    cases.append((json.loads(gc[1]), '4', 'SAVE_EXCURSION_GC_ROOTS'))
    rows = []
    for source, expected, key in cases:
        run = subprocess.run([str(GNU), '-Q', '--batch', '--eval', '(prin1 ' + source + ')'], capture_output=True, text=True, check=True)
        assert run.stdout == expected, (key, run.stdout, expected, run.stderr)
        rows.append(dict(key=key, source=source, answer=run.stdout))
    record = dict(binary=str(GNU), sha256=hashlib.sha256(GNU.read_bytes()).hexdigest(),
                  revision=subprocess.check_output(['git', '-C', str(GNU.parents[1]), 'rev-parse', 'HEAD'], text=True).strip(),
                  version=subprocess.check_output([str(GNU), '--version'], text=True).splitlines()[0], cases=rows)
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    (EVIDENCE / 'gnu.json').write_text(json.dumps(record, indent=2) + '\n')
    print(f'{len(rows)} exact live-GNU excursion answers recorded')


MUTANTS = [
    ('no-point-restore', 'Self::excursion_call(b, GotoChar, &[position]);', 'let _ = position;', 'body_and_point', 'SAVE_EXCURSION_BODY_POINT'),
    ('numeric-point', 'let position = Self::excursion_call(b, MarkerPosition, &[marker]);', 'let position = point;', 'moving_marker_and_left_bias', 'SAVE_EXCURSION_MOVING_LEFT_MARKER'),
    ('no-buffer-restore', 'Self::excursion_call(b, SetBuffer, &[buffer]);', 'let _ = buffer;', 'restores_buffer', 'SAVE_EXCURSION_BUFFER'),
    ('dead-buffer-restored', 'b.code[branch] = Op::Unless(reg, end);', 'b.code[branch] = if branch == dead { Op::Jump(branch + 1) } else { Op::Unless(reg, end) };', 'dead_original_is_not_restored', 'SAVE_EXCURSION_DEAD_BUFFER'),
    ('no-window-point', 'Self::excursion_call(b, SetWindowPoint, &[window, restored]);', 'let _ = restored;', 'selected_window_point', 'SAVE_EXCURSION_WINDOW_POINT'),
    ('forget-original-visibility', 'b.code[branch] = Op::Unless(reg, end);', 'b.code[branch] = if branch == not_shown { Op::Jump(branch + 1) } else { Op::Unless(reg, end) };', 'window_not_originally_showing_buffer', 'SAVE_EXCURSION_WINDOW_NOT_SHOWN'),
    ('forget-current-visibility', 'b.code[branch] = Op::Unless(reg, end);', 'b.code[branch] = if branch == changed { Op::Jump(branch + 1) } else { Op::Unless(reg, end) };', 'window_no_longer_showing_buffer', 'SAVE_EXCURSION_WINDOW_CHANGED'),
]


def mutants():
    original = SOURCE.read_text()
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    rows = []
    try:
        for name, old, new, test, key in MUTANTS:
            assert original.count(old) == 1, name
            SOURCE.write_text(original.replace(old, new))
            run = subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--test', 'save_excursion', test, '--', '--exact', '--nocapture'], cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            (EVIDENCE / (name + '.log')).write_text(run.stdout)
            killed = run.returncode == 101 and f'failed: {key}:' in run.stdout and 'test result: FAILED' in run.stdout
            rows.append(dict(name=name, test=test, assertion=key, rc=run.returncode, killed=killed))
            print(name, 'KILLED' if killed else 'NOT KILLED', flush=True)
            assert killed, (name, run.stdout[-4000:])
    finally:
        SOURCE.write_text(original)
        (EVIDENCE / 'mutants.json').write_text(json.dumps(rows, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--record-gnu', action='store_true')
    parser.add_argument('--mutants', action='store_true')
    args = parser.parse_args()
    if args.record_gnu:
        record()
    if args.mutants:
        mutants()
