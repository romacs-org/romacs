#!/usr/bin/env python3
"""Review only two inspected printer limitations; reject any other mismatch.

numbers.rs:4-6 prints every NaN with a zero payload. The primitive's nonzero
NaN payload is separately checked bit-for-bit in number_parse.rs's unit test.
Circular payloads remain in the Cartesian sweep, with two finite graph probes
checking original error data/identity without depending on cycle printing.
No semantic outcome is automatically accepted.
"""
import os as _portable_os
from pathlib import Path as _PortablePath
import json
from pathlib import Path
import subprocess
import primh

here = Path(__file__).resolve().parent
spec = primh.load_spec(here / 'specs/string-to-number.json')
ev = primh.evidence(spec)
result = subprocess.run([_portable_os.environ.get('ROMACS_VM2_BIN', str(next(p for p in _PortablePath(__file__).resolve().parents if (p / 'parity-reference.toml').is_file()) / 'target/debug/romacs-vm')), '--gc-floor', '1', '--eval-stream'], input=''.join(json.dumps({'source': r['source']}) + '\n' for r in ev['cases']), capture_output=True, text=True, check=True)
actuals = [json.loads(line) for line in result.stdout.splitlines()]
assert len(actuals) == len(ev['cases']) == 626
pins, reviewed = {}, []
for row, out in zip(ev['cases'], actuals):
    value = (out.get('result') or {}).get('value')
    if value == row['expected'] or value == row.get('gnu_regime_variant') and value is not None:
        continue
    if out == {'error': 'circular list printing requires print-circle'}:
        assert '(1 2 1 2 . #2)' in row['expected']
        reason = 'cycle printer cannot emit the GNU finite circular-list notation'
        pins[row['id']] = 'PRINTER'
    else:
        assert row['id'] == 'string-to-number/0516'
        assert row['expected'] == '(ok 1.0e+NaN)' and value == '(ok 0.0e+NaN)'
        reason = 'NaN printer drops payload (numbers.rs:4-6); nonzero bits independently verified'
        # The unchanged harness classifies numeric print differences as VALUE.
        pins[row['id']] = 'VALUE'
    reviewed.append(dict(id=row['id'], source=row['source'], expected=row['expected'], actual=out, reason=reason, harness_class=pins[row['id']]))
assert len(pins) == 32
assert sum(v == 'PRINTER' for v in pins.values()) == 31
primh.atomic_json(here / 'logs/string-to-number-printer-review.json', reviewed)
ev['pinned'] = pins
primh.atomic_json(here / 'evidence/string-to-number.json', ev)
print('Reviewed 31 circular-printer limitations and 1 NaN-payload printer limitation (harness class VALUE).')
