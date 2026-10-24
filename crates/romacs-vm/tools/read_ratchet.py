#!/usr/bin/env python3
"""Owner-accepted read costs against immutable flat Text, never against Guile."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import hashlib
import json
import statistics
from pathlib import Path
import sys
sys.path.insert(0, _portable_os.environ['ROMACS_BENCH_SUPPORT'])
from guard import validate

# Full-precision raw S9/flat medians, from s9-evidence/final/historical/buffer.json
# SHA256 5b1715bd16025f73719de34014d2ccf198145080c1425ce786ec1980983f178c.
LOCATE_NEWLINE_ASCII_MAX_RATIO = 1.2463525479867612
CHAR_AT_ASCII_MAX_RATIO = 1.2188105337899982
LOCATE_NEWLINE_UNICODE_MAX_RATIO = 1.295891322581137
CHAR_AT_UNICODE_MAX_RATIO = 2.305663661441276
# Owner-authorized noise budget, applied once to every row: measured <= cap * 1.02.
# The counter has 36-instruction resolution and a conservative 72-instruction
# slope floor. A zero-tolerance ratchet flaps on noise rather than regressions;
# 2% is above sampling noise and well below the regressions this guard targets.
NOISE_ALLOWANCE = 1.02
ACCEPTED_RATIOS = {
    'append-read-ascii': 1.0,
    'scan-ascii': 1.0,
    'locate-newline-ascii': LOCATE_NEWLINE_ASCII_MAX_RATIO,
    'char-at-ascii': CHAR_AT_ASCII_MAX_RATIO,
    'mutate-scan-ascii': 1.0,
    'append-read-unicode': 1.0,
    'scan-unicode': 1.0,
    'locate-newline-unicode': LOCATE_NEWLINE_UNICODE_MAX_RATIO,
    'char-at-unicode': CHAR_AT_UNICODE_MAX_RATIO,
    'mutate-scan-unicode': 1.0,
}
FLAT_EVIDENCE = Path(str(_PortablePath(_portable_tempfile.gettempdir()) / 'romacs/vm2/s7-evidence/final/control-historical/buffer.json'))
FLAT_EVIDENCE_SHA256 = '39bb09d1577cac6e42b5aabe422accb5997e6b0f8616ab6f5ddf913cfda62330'
FLAT_BINARY_SHA256 = 'dac3f6eeea587409cffcf3586a4768de40c9abb262a33d0965d1533a82ad2e4f'


def checked_costs(report):
    assert report['complete'] and report['resolution'] == 36 and report['slope_floor'] == 72
    rows = {r['name']: r for r in report['rows']}
    assert len(report['rows']) == len(rows) == 10 and rows.keys() == ACCEPTED_RATIOS.keys()
    assert len(report['samples']) == 120
    for sample in report['samples']:
        validate(sample['count'], sample['enabled'], sample['running'])
        assert sample['stdout'] == sample['expected']
    costs = {}
    for name, row in rows.items():
        n = row['n']
        assert n > 0
        for engine in ('vm2', 'guile'):
            samples = [s for s in report['samples'] if s['row'] == name and s['engine'] == engine]
            assert len(samples) == 6
            slopes = []
            for rep in range(3):
                pair = {s['n']: s for s in samples if s['rep'] == rep}
                assert len(pair) == 2 and pair.keys() == {n, 2*n}
                delta = pair[2*n]['count'] - pair[n]['count']
                assert delta > 72, (name, engine, delta)
                slopes.append(delta/n)
            assert row['costs'][engine]['slopes'] == slopes
            assert row['costs'][engine]['median'] == statistics.median(slopes)
        costs[name] = row['costs']['vm2']['median']
    return costs


def qualify(report, flat_path=FLAT_EVIDENCE):
    if not __debug__:
        raise RuntimeError('Python -O is forbidden')
    raw = flat_path.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == FLAT_EVIDENCE_SHA256, 'flat evidence changed'
    flat = json.loads(raw)
    assert flat['sha256'] == FLAT_BINARY_SHA256
    old, new = checked_costs(flat), checked_costs(report)
    assert report['source_hashes'] == flat['source_hashes'], 'workloads changed'
    failures = []
    for name, limit in ACCEPTED_RATIOS.items():
        # Conservative 72-instruction uncertainty; one explicit noise allowance below.
        ratio = (new[name] + 72 / next(r['n'] for r in report['rows'] if r['name'] == name)) / old[name]
        if ratio > limit * NOISE_ALLOWANCE:
            failures.append(f'{name}: {ratio!r} > {limit!r} * {NOISE_ALLOWANCE}')
    assert not failures, '\n'.join(failures)


if __name__ == '__main__':
    import sys
    qualify(json.loads(Path(sys.argv[1]).read_text()))
    print('ten-row frozen-flat recorded-ratio ratchet PASS')
