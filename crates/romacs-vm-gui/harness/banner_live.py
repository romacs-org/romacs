#!/usr/bin/env python3
"""Banners (lisp/ui/romacs-banner.el) move while the main VM is busy: the
render thread draws every pattern's motion, no Lisp runs for it.

A native session shows one banner of each pattern (wave, rain, sparkle,
sweep), each three rows in a window of its own, then a timer holds the
main VM in a busy loop.  Shots taken in that stall must differ (the
banners moved, and nothing else can: no redisplay runs); shots once the
banners are at rest, the main VM free, must not.  The shots are the
patterns' screenshots.

ROMACS_LAYER_ANIMATION=1 runs the path a GPU runs where the rasterizer is
software, as landing_anim_live.py does.  Private HOME/XDG and compositor.

usage: banner_live.py --binary PATH --output DIR [--tools DIR]
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
(require 'romacs-banner)
(defconst banner-live-faces '(romacs-working-running romacs-working-waiting success))
(defun banner-live-show (moving)
  "One window a pattern, its banner MOVING or at rest."
  (when (get-buffer "*Romacs*") (kill-buffer "*Romacs*"))
  (delete-other-windows)
  (let ((first t))
    (dolist (pattern '(wave rain sparkle sweep))
      (unless first (select-window (split-window (get-largest-window) nil (quote below))))
      (setq first nil)
      (let ((buffer (get-buffer-create (format "*banner %s*" pattern))))
        (switch-to-buffer buffer)
        (let ((inhibit-read-only t))
          (erase-buffer)
          (insert (romacs-banner-still pattern 48 3 banner-live-faces 7) "\n" (format "%s" pattern) "\n"))
        (goto-char (point-max))
        (romacs-banner-show (intern (format "banner-live-%s" pattern)) buffer (point-min) pattern 48 3
                            banner-live-faces moving 7))))
  (balance-windows)
  (redisplay)
  ;; Each banner's block in frame pixels, for the shots to compare.
  (with-temp-file (expand-file-name "blocks" temporary-file-directory)
    (dolist (window (window-list))
      (pcase-let ((`(,left ,top . ,_) (window-inside-pixel-edges window)))
        (insert (format "%s %dx%d+%d+%d\n" (buffer-name (window-buffer window))
                        (* 48 (frame-char-width)) (* 3 (line-pixel-height)) left top))))))
(run-at-time 3 nil #'banner-live-show t)
(defun banner-live-busy (file seconds)
  "Write FILE, then hold the main VM SECONDS, then write FILE.done."
  (write-region "" nil file nil 'silent)
  (let ((end (+ (float-time) seconds))) (while (< (float-time) end)))
  (write-region "" nil (concat file ".done") nil 'silent))
(run-at-time 6 nil #'banner-live-busy (expand-file-name "busy" temporary-file-directory) 6)
(run-at-time 16 nil (lambda () (banner-live-show nil)
                      (write-region "" nil (expand-file-name "rest" temporary-file-directory) nil 'silent)))
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
    with live.session(a.binary, a.output, prefix='banner-', init=INIT, env={'ROMACS_LAYER_ANIMATION': '1'},
                      tools=a.tools and str(a.tools), report=report) as session:
        tmp = Path(session.env['TMPDIR'])

        def shot(name):
            path = a.output / f'{name}.png'
            subprocess.run([tool('grim'), str(path)], env=session.env, check=True, timeout=10)
            return path

        def differ(x, y, region):
            """Pixels that differ between two shots in REGION (WxH+X+Y)."""
            out = subprocess.run([tool('magick'), 'compare', '-metric', 'AE', f'{x}[{region}]', f'{y}[{region}]', 'null:'],
                                 capture_output=True, text=True)
            return float(out.stderr.split()[0] or 0)

        def blocks():
            return dict(line.rsplit(' ', 1) for line in (tmp / 'blocks').read_text().splitlines())

        fio.wait(lambda: (tmp / 'busy').exists(), 60, 'the busy loop', session.process)
        time.sleep(0.5)
        busy = [shot(f'busy-{i}') for i in range(4) if not time.sleep(0.6)]
        busy_still = not (tmp / 'busy.done').exists()
        moving = blocks()
        # Each banner's own block moved between every two shots.
        moved = {name: min(differ(x, y, region) for x, y in zip(busy, busy[1:])) for name, region in moving.items()}
        fio.wait(lambda: (tmp / 'rest').exists(), 60, 'the banners at rest', session.process)
        time.sleep(1.0)
        rest = [shot(f'rest-{i}') for i in range(3) if not time.sleep(0.6)]
        rested = max(differ(x, y, region) for x, y in zip(rest, rest[1:]) for region in blocks().values())
        report['checks'] = [{'check': 'every banner moves while the main VM is busy',
                             'ok': busy_still and len(moved) == 4 and min(moved.values()) > 0,
                             'detail': {'least_changed_pixels': moved, 'busy_throughout': busy_still}},
                            {'check': 'at rest they stand still', 'ok': rested == 0, 'detail': {'changed_pixels': rested}}]
    ok = all(c['ok'] for c in report['checks']) and report['checks']
    report['status'] = 'PASS' if ok else 'FAIL'
    (a.output / 'report.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    return 0 if ok else 1


if __name__ == '__main__':
    raise SystemExit(main())
