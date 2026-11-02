#!/usr/bin/env python3
"""Pane chrome in a live session (DIVERGENCES.md VM2-PANE-CHROME): the
root frame's windows drawn as panes, one, two and three of them.

One private headless labwc and one `romacs-vm-gui --loadup'.  The
compositor renders on the host GPU when ROMACS_HARNESS_WLR_ENV gives it
one (romacs-gpu-env.sh), and Romacs then draws on that GPU too; --scale
scales the output (fractional Wayland scaling).  An init drives the
session: it records `romacs-chrome-state' and every window's edges, then
splits right, then below, recording each, and exits.  After each record
a screenshot (grim) is probed:

  divider  a pixel in the right divider of the first window is the frame
           background (modern only);
  focus    a pixel on the selected window's left edge is the
           `romacs-pane-focus' colour (modern only);
  corner   the selected window's top-left corner pixel, outside its
           rounded corner, is the frame background (modern only);

and the run checks that `romacs-chrome-state' reports gap 6, radius 6,
dim 0.12 (or classic and 0 with --classic) and that the exit's frame
counters (ROMACS_FRAME_STATS) drew pane chrome quads (none with
--classic).  With --slowdown N the window animations run N times
longer and a screenshot is taken a third into the second split.

With --gpu-frames N (at least 200) the three panes then show N edits,
one every 50 ms, each a new presentation (the cursor does not blink),
with the root pass's GPU
timestamps on (ROMACS_GPU_FRAME_TIMING): the report gives the median GPU
time of the root glyph pass, which draws the panes too, over the last
200 presentations the present trace records (each one's first draw),
the measure to compare between a modern and a --classic run, which
fails unless each of the 200 has a sample.

With --light the default face is dark on light (#202020 on #f0f0f0),
where a corner blended wrongly shows.

usage: chrome_live.py --binary BIN --output DIR [--classic] [--scale 1.5]
       [--slowdown 8] [--mode 1600x1000] [--gpu-frames 200] [--light]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

import live_oracle as live

INIT = r'''
(defvar chrome-live-dir (getenv "CHROME_LIVE_DIR"))
(defun chrome-live--hex (color)
  (and color (apply #'format "#%02x%02x%02x" (mapcar (lambda (v) (ash v -8)) (color-values color)))))
(defun chrome-live--note (name)
  "Record the chrome and the windows' edges in NAME, a JSON file."
  (let ((state (romacs-chrome-state)))
    (with-temp-file (expand-file-name (concat name ".json") chrome-live-dir)
      (insert (format "{\"classic\": %s, \"gap\": %d, \"radius\": %d, \"dim\": %s, \"right_divider\": %d, \"internal_border\": %d, \"focus\": %S, \"background\": %S, \"windows\": [%s]}"
                      (if (plist-get state :classic) "true" "false") (plist-get state :gap) (plist-get state :pane-radius)
                      (plist-get state :pane-dim) (plist-get state :right-divider) (plist-get state :internal-border)
                      (or (plist-get state :focus) "") (chrome-live--hex (face-background 'default nil t))
                      (mapconcat (lambda (w) (format "{\"edges\": [%s], \"selected\": %s}"
                                                     (mapconcat #'number-to-string (window-edges w nil nil t) ", ")
                                                     (if (eq w (selected-window)) "true" "false")))
                                 (window-list nil 'never (frame-first-window)) ", "))))))
'''

STEPS = r'''
(run-at-time 4 nil (lambda () (chrome-live--note "1")))
(run-at-time 7 nil (lambda () (split-window-right) (chrome-live--note "2")))
(run-at-time 10 nil (lambda () (split-window-below) (chrome-live--note "3")))
'''
LOAD = r'''
(blink-cursor-mode -1)
(run-at-time 13 nil (lambda ()
  (set-window-buffer (frame-first-window) (get-buffer-create "load"))
  (let ((n 0) timer)
    (setq timer (run-at-time 0.5 0.05 (lambda ()
      (with-current-buffer "load" (goto-char (point-max)) (insert (if (zerop (% n 60)) "\n" "x")))
      (when (>= (setq n (1+ n)) {frames}) (cancel-timer timer) (kill-emacs 0))))))))
'''


def wait(pred, timeout, what, proc):
    end = time.time() + timeout
    while time.time() < end:
        if proc is not None and proc.poll() is not None:
            raise RuntimeError(f'binary exited while waiting for {what}')
        if pred():
            return
        time.sleep(.05)
    raise RuntimeError(f'timeout: {what}')


def pixel(png: Path, x: int, y: int) -> str:
    """The #rrggbb of PNG's pixel at X, Y."""
    out = subprocess.run(['magick', str(png), '-crop', f'1x1+{x}+{y}', '-format', '#%[hex:p{0,0}]', 'info:'],
                         capture_output=True, text=True, check=True)
    return out.stdout.strip().lower()[:7]


def near(a: str, b: str, slack: int = 3) -> bool:
    return all(abs(int(a[i:i + 2], 16) - int(b[i:i + 2], 16)) <= slack for i in (1, 3, 5))


MEASURED = 200


def gpu_times(stderr: str, trace: Path, frames: int) -> dict:
    """The median GPU ns of the root glyph pass, which draws the panes
    too, over the last FRAMES presentations the root frame presented (the
    present trace), each the first draw of it; MISSING those without a
    sample."""
    first = {}
    for line in stderr.splitlines():
        fields = dict(re.findall(r'(\w+)=(\S+)', line)) if 'GPU duration' in line else {}
        if fields.get('frame') == '1' and 'presentation' in fields and 'duration_ns' in fields:
            first.setdefault(int(fields['presentation']), int(fields['duration_ns']))
    presented = []
    for line in trace.read_text().splitlines() if trace.exists() else []:
        record = json.loads(line)
        if record['stage'] == 'presented' and record['frame'] == 1 and record['presentation'] not in presented:
            presented.append(record['presentation'])
    measured = presented[-frames:]
    drawn = sorted(first[p] for p in measured if p in first)
    return {'presentations': len(measured), 'missing': len(measured) - len(drawn),
            'median_ns': drawn[len(drawn) // 2] if drawn else None}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', required=True, type=Path)
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--classic', action='store_true')
    ap.add_argument('--scale', type=float, default=1.0)
    ap.add_argument('--slowdown', type=float, default=1.0)
    ap.add_argument('--mode', default='1600x1000')
    ap.add_argument('--timeout', type=float, default=120)
    ap.add_argument('--gpu-frames', type=int, default=0)
    ap.add_argument('--light', action='store_true')
    ap.add_argument('--labwc', default=live.LABWC)
    ap.add_argument('--wlr-randr', default=live.RANDR)
    args = ap.parse_args()
    if args.gpu_frames and args.gpu_frames < MEASURED:
        ap.error(f'--gpu-frames measures the last {MEASURED} presentations')
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'classic': args.classic, 'scale': args.scale, 'slowdown': args.slowdown, 'checks': {}}
    checks = report['checks']
    comp = kb = proc = None
    try:
        with tempfile.TemporaryDirectory(prefix='chr-', dir=os.environ.get('XDG_RUNTIME_DIR')) as tmp:
            root = Path(tmp)
            root.chmod(0o700)
            home, config, notes = root / 'home', root / 'config', root / 'notes'
            for d in (home, config, notes):
                d.mkdir()
            (config / 'rc.xml').write_text(live.RC_XML)
            (home / 'config' / 'romacs').mkdir(parents=True)
            init = INIT
            if args.light:
                init += '(set-face-attribute \'default nil :foreground "#202020" :background "#f0f0f0")\n'
            if args.classic:
                init += '(romacs-classic-chrome 1)\n'
            if args.slowdown != 1:
                init += f'(setopt romacs-window-animations-slowdown {args.slowdown})\n'
            end = LOAD.replace('{frames}', str(args.gpu_frames)) if args.gpu_frames else '(run-at-time 13 nil (lambda () (kill-emacs 0)))\n'
            (home / 'config' / 'romacs' / 'init.el').write_text(init + STEPS + end)
            stats = out / 'frame-stats.txt'
            if stats.exists():
                stats.unlink()
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '15',
                   'ROMACS_FRAME_STATS': str(stats), 'CHROME_LIVE_DIR': str(notes)}
            if args.gpu_frames:
                env.update(ROMACS_GPU_FRAME_TIMING='1', ROMACS_PRESENT_TRACE=str(out / 'trace.jsonl'))
            for k in ('CARGO_HOME', 'ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR', 'ROMACS_WORLD'):
                if k in os.environ:
                    env[k] = os.environ[k]
            wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
            report['compositor_renderer'] = wlr.get('WLR_RENDERER', 'pixman')
            log_path = out / 'binary.stderr'
            with (out / 'compositor.log').open('w') as clog, log_path.open('w') as log:
                comp = subprocess.Popen([args.labwc, '-C', str(config)], env=env | wlr, stdout=clog, stderr=clog)
                wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'compositor', comp)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([args.wlr_randr, '--output', 'HEADLESS-1', '--custom-mode', args.mode,
                                '--scale', str(args.scale)], env=env, check=True, timeout=15)
                kb = subprocess.Popen(['/usr/bin/wtype', '-s', '600000', '-k', 'Shift_L'], env=env,
                                      stdout=log, stderr=log)
                time.sleep(.3)
                proc = subprocess.Popen([str(binary), '--loadup'], env=env, cwd=home,
                                        stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def shot(name):
                    path = out / f'{name}.png'
                    subprocess.run(['/usr/bin/grim', str(path)], env=env, check=True, timeout=30)
                    return path

                for step in ('1', '2', '3'):
                    note = notes / f'{step}.json'
                    wait(note.exists, args.timeout, f'record {step}', proc)
                    if step == '3' and args.slowdown != 1:
                        time.sleep(.2 * args.slowdown / 3)
                        shot(f'panes-{step}-mid')
                    time.sleep(1.2)
                    png = shot(f'panes-{step}')
                    state = json.loads(note.read_text())
                    report[f'state-{step}'] = state
                    first = state['windows'][0]
                    selected = next(w for w in state['windows'] if w['selected'])
                    s = args.scale
                    if not args.classic and step != '1':
                        left, top, right, bottom = first['edges']
                        dx, dy = round((right - state['right_divider'] / 2) * s), round((top + bottom) / 2 * s)
                        checks[f'divider-{step}'] = [pixel(png, dx, dy), state['background']]
                    if not args.classic:
                        left, top, right, bottom = selected['edges']
                        fx, fy = round(left * s), round((top + bottom) / 2 * s)
                        checks[f'focus-{step}'] = [pixel(png, fx, fy), state['focus']]
                        checks[f'corner-{step}'] = [pixel(png, round(left * s), round(top * s)), state['background']]
                wait(lambda: proc.poll() is not None, 30 + args.gpu_frames * .1, 'exit', None)
            if args.gpu_frames:
                report['gpu'] = gpu_times(log_path.read_text(errors='replace'), out / 'trace.jsonl', MEASURED)
            text = stats.read_text() if stats.exists() else ''
            quads = re.search(r'chrome_divider_quads: (\d+)', text)
            report['chrome_divider_quads'] = int(quads.group(1)) if quads else None
            state = report['state-1']
            wanted = (True, 0, 0, 0.0) if args.classic else (False, 6, 6, 0.12)
            ok = (state['classic'], state['gap'] if not args.classic else state['right_divider'], state['radius'],
                  float(state['dim'])) == wanted
            ok = ok and (report['chrome_divider_quads'] == 0 if args.classic else (report['chrome_divider_quads'] or 0) > 0)
            for name, (got, want) in checks.items():
                ok = ok and near(got, want)
            if args.gpu_frames:
                gpu = report['gpu']
                ok = ok and gpu['presentations'] == MEASURED and gpu['missing'] == 0
            report['status'] = 'PASS' if ok else 'FAIL'
    except Exception as error:
        report.update(status='FAIL', error=repr(error))
    finally:
        for p in (proc, kb, comp):
            if p is not None and p.poll() is None:
                p.terminate()
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()
        (out / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
        print(json.dumps(report))
    return 0 if report.get('status') == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
