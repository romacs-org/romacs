#!/usr/bin/env python3
"""The working thread (lisp/ui/romacs-working.el) moves while the main VM
is busy: the render thread draws its motion, no Lisp runs for it.

A native session shows a running thread at the top of a buffer, then a
timer holds the main VM in a busy loop for six seconds.  Shots taken in
that stall must differ (the thread moved, and nothing else can: no
redisplay runs); shots of an idle thread, the main VM free, must not.

ROMACS_LAYER_ANIMATION=1 runs the path a GPU runs where the rasterizer is
software, as landing_anim_live.py does.  Private HOME/XDG and compositor.

usage: working_live.py --binary PATH --output DIR [--tools DIR]
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

import fileio_live as fio
import live_oracle as live

INIT = r'''
(blink-cursor-mode -1)
(require 'romacs-working)
;; Once the start is over, the thread alone on the screen: no landing.
(defun working-live-show ()
  (when (get-buffer "*Romacs*") (kill-buffer "*Romacs*"))
  (let ((buffer (get-buffer-create "*working*")))
    (switch-to-buffer buffer)
    (delete-other-windows)
    (insert (romacs-working-still 24 'running) " running\n")
    (romacs-working-show 'harness buffer (point-min) 24 'running)))
(run-at-time 3 nil #'working-live-show)
(defun working-live-busy (file seconds)
  "Write FILE, then hold the main VM SECONDS, then write FILE.done."
  (write-region "" nil file nil 'silent)
  (let ((end (+ (float-time) seconds))) (while (< (float-time) end)))
  (write-region "" nil (concat file ".done") nil 'silent))
(run-at-time 6 nil #'working-live-busy (expand-file-name "busy" temporary-file-directory) 6)
(run-at-time 16 nil (lambda () (romacs-working-set 'harness 'idle)
                      (write-region "" nil (expand-file-name "idle" temporary-file-directory) nil 'silent)))
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tools', type=Path)
    a = parser.parse_args()
    a.output.mkdir(parents=True)
    tool = lambda name: str(a.tools / name) if a.tools else name
    report = {'checks': []}
    with live.session(a.binary, a.output, prefix='working-', init=INIT, env={'ROMACS_LAYER_ANIMATION': '1'},
                      tools=a.tools and str(a.tools), report=report) as session:
        tmp = Path(session.env['TMPDIR'])

        def shot(name):
            path = a.output / f'{name}.png'
            subprocess.run([tool('grim'), str(path)], env=session.env, check=True, timeout=10)
            return path

        def differ(x, y):
            """Pixels that differ in the thread's cells, the first of the window's first row."""
            row = '240x24+10+8'
            out = subprocess.run([tool('magick'), 'compare', '-metric', 'AE', f'{x}[{row}]', f'{y}[{row}]', 'null:'],
                                 capture_output=True, text=True)
            return float(out.stderr.split()[0] or 0)

        fio.wait(lambda: (tmp / 'busy').exists(), 60, 'the busy loop', session.process)
        time.sleep(0.5)
        busy = [shot(f'busy-{i}') for i in range(4) if not time.sleep(0.6)]
        busy_still = not (tmp / 'busy.done').exists()
        moved = max(differ(x, y) for x, y in zip(busy, busy[1:]))
        fio.wait(lambda: (tmp / 'idle').exists(), 60, 'the idle thread', session.process)
        time.sleep(1.0)
        idle = [shot(f'idle-{i}') for i in range(3) if not time.sleep(0.6)]
        rested = max(differ(x, y) for x, y in zip(idle, idle[1:]))
        report['checks'] = [{'check': 'moves while the main VM is busy', 'ok': busy_still and moved > 0,
                             'detail': {'changed_pixels': moved, 'busy_throughout': busy_still}},
                            {'check': 'idle stands still', 'ok': rested == 0, 'detail': {'changed_pixels': rested}}]
    ok = all(c['ok'] for c in report['checks']) and report['checks']
    report['status'] = 'PASS' if ok else 'FAIL'
    (a.output / 'report.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    return 0 if ok else 1


if __name__ == '__main__':
    raise SystemExit(main())
