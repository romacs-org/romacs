#!/usr/bin/env python3
"""O1b-3 pane layers on a real compositor: the display runtime's
pane_layers_probe shows a root naming one foreign pane, then posts a real
pane picture for it (laid out in an owner's font slot) carrying a working
thread.  The pane draws as a layer, and the thread moves in it.

Checks, on grim shots of a private headless labwc:
  - the picture draws: the shot changes when it is posted;
  - it settles: the probe's sink reports `settled v=1`;
  - the thread moves: successive shots differ, the root static;
  - nothing else moves: with --still the same shots are equal.

usage: pane_layers_live.py --probe PATH --output DIR [--tools DIR]
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

import fileio_live as fio
import live_oracle as live


def run(a, name, still, tool):
    out = a.output / name
    out.mkdir()
    args = ['--seconds', '30'] + (['--still'] if still else [])
    with live.session(a.probe, out, prefix='panes-', args=args, tools=a.tools and str(a.tools),
                      env={'ROMACS_LAYER_ANIMATION': '1'}) as session:
        log = session.log

        def shot(label):
            path = out / f'{label}.png'
            subprocess.run([tool('grim'), str(path)], env=session.env, check=True, timeout=10)
            return path

        def changed(x, y):
            # Whole shots: the root and the window stand still, so any change is the pane's.
            result = subprocess.run([tool('magick'), 'compare', '-metric', 'AE', str(x), str(y), 'null:'],
                                    capture_output=True, text=True)
            return float(result.stderr.split()[0] or 0)

        fio.wait(lambda: 'activated' in log.read_text(), 60, 'the root', session.process)
        time.sleep(0.5)
        empty = shot('root')
        fio.wait(lambda: 'settled v=1' in log.read_text(), 30, 'the picture settled', session.process)
        time.sleep(1.0)
        shots = [shot(f'pane-{i}') for i in range(4) if not time.sleep(0.5)]
        return {'drawn': changed(empty, shots[0]), 'moved': max(changed(x, y) for x, y in zip(shots, shots[1:]))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tools', type=Path)
    a = parser.parse_args()
    a.output.mkdir(parents=True)
    tool = lambda name: str(a.tools / name) if a.tools else name
    moving, still = run(a, 'moving', False, tool), run(a, 'still', True, tool)
    checks = [{'check': 'the picture draws in its rect', 'ok': moving['drawn'] > 0, 'detail': moving},
              {'check': 'the working thread moves in the pane', 'ok': moving['moved'] > 0, 'detail': moving},
              {'check': 'a still thread leaves the pane still', 'ok': still['drawn'] > 0 and still['moved'] == 0,
               'detail': still}]
    ok = all(check['ok'] for check in checks)
    report = {'status': 'PASS' if ok else 'FAIL', 'checks': checks}
    (a.output / 'report.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    return 0 if ok else 1


if __name__ == '__main__':
    raise SystemExit(main())
