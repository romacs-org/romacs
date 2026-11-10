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

  line     a pixel in the middle of the status bar, below its top, is the
           `mode-line-active' background and its left edge the accent
           (modern only; DIVERGENCES.md VM2-STATUS-BAR);
  bands    no window has a mode-line band: the bar is the mode line;

and the run checks that `romacs-chrome-state' reports gap 6, radius 6,
dim 0.12 (or classic, 0) and status bar 1 in either look, and that the
exit's frame counters (ROMACS_FRAME_STATS) drew pane chrome quads (none
with --classic) and status-bar bands.

With --scenes the session then shows, each in a screenshot: a file of
a Git repository (vc), that buffer modified, the buffer in
`flymake-mode' with an error and a warning, an M-x prompt, and three
tall toasts: every one shown ends above the status bar
(`romacs-chrome-bottom') and the bar's pixel probe still finds its
background (on a short --mode one toast waits hidden); then a narrow
window, and a bottom side window whose buffer sets its own line
(keymap-popup's), which the bar leaves out: it shows the selected window's alone.  With --slowdown N the window animations run N times
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

With --themes A,B (by default, on a plain run with none of --classic,
--scenes, --gpu-frames, --theme or --light, DEFAULT_THEMES; `--themes ""'
for none) the three panes then show each theme in turn, every
other theme disabled first (`load-theme'), each recorded and probed as
above: the focus edge must show the colour `romacs-chrome-state' reports
for that theme, and every theme's focus colour must differ from the
others' and from the one before any theme (romacs-chrome.el
`romacs-chrome-derived-faces').

usage: chrome_live.py --binary BIN --output DIR [--classic] [--scale 1.5]
       [--slowdown 8] [--mode 1600x1000] [--gpu-frames 200] [--light]
       [--themes catppuccin-latte,tokyonight-night | --scenes]
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

# Shipped themes with different focus colours: the first two box the mode
# line, so the bar is taller and draws a box; the last does not.
DEFAULT_THEMES = 'catppuccin-latte,tokyonight-night,omarchy-gruvbox'

INIT = r'''
(defvar chrome-live-dir (getenv "CHROME_LIVE_DIR"))
(defun chrome-live--hex (color)
  (and color (apply #'format "#%02x%02x%02x" (mapcar (lambda (v) (ash v -8)) (color-values color)))))
(defun chrome-live--note-laid-out (name)
  "Note NAME once the host has redisplayed since the mark: a theme's rows take
effect in the layout, which the bar's height and the windows' edges report."
  (if (romacs--redisplay-since-mark) (chrome-live--note name)
    (run-with-timer 0.05 nil #'chrome-live--note-laid-out name)))
(defun chrome-live--note (name)
  "Record the chrome and the windows' edges in NAME, a JSON file."
  (let ((state (romacs-chrome-state)))
    (with-temp-file (expand-file-name (concat name ".json") chrome-live-dir)
      (insert (format "{\"classic\": %s, \"gap\": %d, \"radius\": %d, \"dim\": %s, \"right_divider\": %d, \"internal_border\": %d, \"focus\": %S, \"background\": %S, \"windows\": [%s], \"status_bar\": %d, \"bar_top\": %d, \"bar_background\": %S, \"native_width\": %d, \"border\": %d, \"pill\": [%S, %S], \"state_pill\": [%S, %S], \"char_width\": %d, \"bar_height\": %d, \"inactive\": %S}"
                      (if (plist-get state :classic) "true" "false") (plist-get state :gap) (plist-get state :pane-radius)
                      (plist-get state :pane-dim) (plist-get state :right-divider) (plist-get state :internal-border)
                      (or (plist-get state :focus) "") (chrome-live--hex (face-background 'default nil t))
                      (mapconcat (lambda (w) (format "{\"edges\": [%s], \"selected\": %s, \"mode_line\": %d}"
                                                     (mapconcat #'number-to-string (window-edges w nil nil t) ", ")
                                                     (if (eq w (selected-window)) "true" "false")
                                                     (window-mode-line-height w)))
                                 (window-list nil 'never (frame-first-window)) ", ")
                      (plist-get state :status-bar)
                      (if (eq romacs-status-bar-position 'top) (- (romacs-chrome-top) (romacs-status-bar-height)) (romacs-chrome-bottom))
                      (chrome-live--hex (face-background 'mode-line-active nil t))
                      (frame-native-width) (frame-internal-border-width)
                      (or (chrome-live--hex (face-foreground 'mode-line-active nil t)) "")
                      (or (chrome-live--hex (face-background 'romacs-mode-line-pill nil t)) "")
                      (or (chrome-live--hex (face-foreground 'romacs-mode-line-state-pill nil t)) "")
                      (or (chrome-live--hex (face-background 'romacs-mode-line-state-pill nil t)) "")
                      (frame-char-width) (romacs-status-bar-height)
                      (or (chrome-live--hex (face-background 'mode-line-inactive nil t)) ""))))))
'''

STEPS = r'''
(run-at-time 4 nil (lambda () (chrome-live--note "1")))
(run-at-time 7 nil (lambda () (split-window-right) (chrome-live--note "2")))
(run-at-time 10 nil (lambda () (split-window-below) (chrome-live--note "3")))
'''
THEME = r'''
(run-at-time {at} nil (lambda () (mapc #'disable-theme custom-enabled-themes) (load-theme '{theme} t)
                        (romacs--redisplay-since-mark t) (chrome-live--note-laid-out "{theme}")))
'''
SCENES = r'''
(setq trusted-content :all)
(defun chrome-live-flymake (report &rest _)
  (funcall report (list (flymake-make-diagnostic (current-buffer) 1 3 :error "an error")
                        (flymake-make-diagnostic (current-buffer) 4 6 :warning "a warning"))))
(run-at-time 13 nil (lambda () (find-file (expand-file-name "repo/a.el" "~")) (chrome-live--note "vc")))
(run-at-time 16 nil (lambda () (with-current-buffer "a.el" (goto-char (point-max)) (insert ";; more\n"))
                      (chrome-live--note "modified")))
(run-at-time 19 nil (lambda () (with-current-buffer "a.el"
                                 (setq-local flymake-diagnostic-functions '(chrome-live-flymake))
                                 (flymake-mode 1) (flymake-start))
                      (run-at-time 1 nil (lambda () (chrome-live--note "flymake")))))
(run-at-time 23 nil (lambda () (chrome-live--note "prompt")))
(run-at-time 28 nil (lambda ()
  (dotimes (i 3) (romacs-layer-toast (format "toast %d\n%s" i (string-join (make-list 5 "a toast line") "\n"))))
  (run-at-time 1 nil (lambda ()
    (with-temp-file (expand-file-name "toasts.json" chrome-live-dir)
      (insert (format "{\"bottom\": %d, \"toasts\": [%s]}" (romacs-chrome-bottom)
                      (mapconcat (lambda (toast)
                                   (let ((f (romacs-layer--frame (car toast))))
                                     (format "[%d, %d, %s]" (cdr (frame-position f)) (+ (cdr (frame-position f)) (frame-pixel-height f))
                                             (if (frame-visible-p f) "true" "false"))))
                                 romacs-layer--toasts ", "))))))))
(run-at-time 32 nil (lambda ()
  (mapc #'romacs-layer--toast-expire (mapcar #'car romacs-layer--toasts))
  (delete-other-windows)
  (switch-to-buffer "a.el")
  (select-window (split-window-right -20))
  (chrome-live--note "narrow")))
(run-at-time 35 nil (lambda ()
  (delete-other-windows)
  (switch-to-buffer "a.el")
  (with-current-buffer (get-buffer-create "*keymap-popup*")
    (erase-buffer) (insert "Navigate   n  Nodes\n           t  Thema\n\nq  Dismiss menu")
    (setq-local mode-line-format '("q  " "Gnosis Dashboard")))
  (display-buffer-in-side-window (get-buffer "*keymap-popup*") '((side . bottom) (window-height . 6)))
  (chrome-live--note "popup")))
(run-at-time 38 nil (lambda ()
  (delete-other-windows)
  (switch-to-buffer (get-buffer-create "chip.el"))
  (emacs-lisp-mode)
  (setq eval-expression-debug-on-error nil)
  (insert "(+ 1 2)\n(car 1)")
  (goto-char (point-min)) (end-of-line)
  (chrome-live--note "eval")))
(run-at-time 43 nil (lambda ()
  ;; Gnosis's dashboard menu (harness/keymap_popup_fixture.el, submenus
  ;; and all) as keymap-popup's Romacs panel.
  (load @FIXTURE@ nil t)
  (delete-other-windows)
  (switch-to-buffer (get-buffer-create "*Gnosis Dashboard*"))
  (special-mode)
  (keymap-popup keymap-popup-fixture-map)
  (chrome-live--note "keymap")))
'''
LOAD = r'''
(blink-cursor-mode -1)
(run-at-time {at} nil (lambda ()
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


def contrast(a: str, b: str) -> float:
    """WCAG 2 contrast of two #rrggbb colours."""
    def luminance(hex_):
        c = [int(hex_[i:i + 2], 16) / 255 for i in (1, 3, 5)]
        c = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in c]
        return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    x, y = luminance(a), luminance(b)
    return (max(x, y) + 0.05) / (min(x, y) + 0.05)


def near(a: str, b: str, slack: int = 3) -> bool:
    if not (a.startswith('#') and b.startswith('#')):
        return a == b
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
    ap.add_argument('--theme', help='a theme to load in the init (a real light theme: catppuccin-latte)')
    ap.add_argument('--themes', help='comma-separated themes to load in turn after the splits')
    ap.add_argument('--scenes', action='store_true')
    ap.add_argument('--top', action='store_true', help="the status bar at the top (`romacs-status-bar-position')")
    ap.add_argument('--labwc', default=live.LABWC)
    ap.add_argument('--wlr-randr', default=live.RANDR)
    args = ap.parse_args()
    if args.gpu_frames and args.gpu_frames < MEASURED:
        ap.error(f'--gpu-frames measures the last {MEASURED} presentations')
    variant = args.classic or args.scenes or args.gpu_frames or args.theme or args.light
    given = ('' if variant else DEFAULT_THEMES) if args.themes is None else args.themes
    themes = [t for t in given.split(',') if t]
    if args.classic and themes:
        ap.error('--themes probes the focus edge, which --classic does not draw')
    if args.scenes and themes:
        ap.error('--scenes and --themes each take the session after the splits; give one')
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
            if args.theme:
                init += f"(load-theme '{args.theme} t)\n"
            if args.classic:
                init += '(romacs-classic-chrome 1)\n'
            if args.top:
                init += "(setopt romacs-status-bar-position 'top)\n"
            if args.slowdown != 1:
                init += f'(setopt romacs-window-animations-slowdown {args.slowdown})\n'
            for i, theme in enumerate(themes):
                init += THEME.replace('{at}', str(13 + 3 * i)).replace('{theme}', theme)
            at = str(13 + 3 * len(themes))
            end = (LOAD.replace('{frames}', str(args.gpu_frames)).replace('{at}', at) if args.gpu_frames
                   else f'(run-at-time {at} nil (lambda () (kill-emacs 0)))\n')
            if args.scenes:
                end = SCENES.replace('@FIXTURE@', json.dumps(str(Path(__file__).resolve().parent / 'keymap_popup_fixture.el'))) + '(run-at-time 52 nil (lambda () (kill-emacs 0)))\n'
                repo = home / 'repo'
                repo.mkdir()
                (repo / 'a.el').write_text(';;; a.el --- a file  -*- lexical-binding: t -*-\n(defun a () 1)\n')
                git = ['git', '-C', str(repo), '-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false']
                for command in (['init', '-q', '-b', 'main'], ['add', 'a.el'], ['commit', '-q', '-m', 'a']):
                    subprocess.run(git + command, check=True, env={'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home)})
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

                for step in ['1', '2', '3'] + themes:
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
                        # The line the selected window's mode line is, or
                        # the bar: its background in the middle, and the
                        # accent block at its left edge.
                        x0, x1, top, height = state['border'], state['native_width'] - state['border'], state['bar_top'], state['bar_height']
                        mid = round((top + height / 2) * s)
                        # The middle of the line, above the identity's pill.
                        checks[f'line-{step}'] = [pixel(png, round((x0 + x1) / 2 * s), round((top + 2) * s)), state['bar_background']]
                        checks[f'accent-{step}'] = [pixel(png, round((x0 + 1) * s), mid), state['focus']]
                        # No window draws a band of its own: the bar is the mode line.
                        checks[f'bands-{step}'] = [str(sum(w['mode_line'] for w in state['windows'])), '0']
                for scene in (('vc', 'modified', 'flymake', 'prompt') if args.scenes else ()):
                    note = notes / f'{scene}.json'
                    wait(note.exists, args.timeout, f'record {scene}', proc)
                    if scene == 'prompt':
                        subprocess.run(['/usr/bin/wtype', '-M', 'alt', 'x', '-m', 'alt'], env=env, timeout=10)
                    time.sleep(1.5)
                    shot(f'scene-{scene}')
                    report[f'state-{scene}'] = json.loads(note.read_text())
                    if scene == 'prompt':
                        subprocess.run(['/usr/bin/wtype', '-M', 'ctrl', 'g', '-m', 'ctrl'], env=env, timeout=10)
                if args.scenes:
                    toasts = notes / 'toasts.json'
                    wait(toasts.exists, args.timeout, 'toasts', proc)
                    time.sleep(1.5)
                    png = shot('scene-toasts')
                    found = json.loads(toasts.read_text())
                    report['toasts'] = found
                    shown = [t for t in found['toasts'] if t[2]]
                    checks['toasts-above-lines'] = ['ok' if shown and all(t[1] <= found['bottom'] for t in shown) else 'covered', 'ok']
                    if not args.classic:
                        # The selected line (or bar) still shows under them.
                        state = report['state-3']
                        lx, ly = state['native_width'] / 2, state['bar_top'] + 2
                        checks['line-under-toasts'] = [pixel(png, round(lx * args.scale), round(ly * args.scale)), state['bar_background']]
                    # A narrow window's line: its segments dropped by priority.
                    narrow = notes / 'narrow.json'
                    wait(narrow.exists, args.timeout, 'narrow', proc)
                    time.sleep(1.5)
                    shot('scene-narrow')
                    report['state-narrow'] = json.loads(narrow.read_text())
                    # A popup's own line stays out of the bar.
                    popup = notes / 'popup.json'
                    wait(popup.exists, args.timeout, 'popup', proc)
                    time.sleep(1.5)
                    shot('scene-popup')
                    report['state-popup'] = json.loads(popup.read_text())
                    # An evaluation's value, then an error, in a chip after the sexp.
                    evaluate = notes / 'eval.json'
                    wait(evaluate.exists, args.timeout, 'eval', proc)
                    time.sleep(.5)
                    ctrl = lambda *keys: subprocess.run(['/usr/bin/wtype', '-M', 'ctrl', *[k for key in keys for k in ('-k', key)], '-m', 'ctrl'],
                                                        env=env, timeout=10)
                    ctrl('x', 'e')
                    time.sleep(1)
                    shot('scene-eval')
                    ctrl('n', 'e')
                    ctrl('x', 'e')
                    time.sleep(1)
                    shot('scene-eval-error')
                    # keymap-popup's menu as Romacs's bottom panel.
                    keymap = notes / 'keymap.json'
                    wait(keymap.exists, args.timeout, 'keymap', proc)
                    time.sleep(1.5)
                    shot('scene-keymap')
                wait(lambda: proc.poll() is not None, 30 + args.gpu_frames * .1, 'exit', None)
            if args.gpu_frames:
                report['gpu'] = gpu_times(log_path.read_text(errors='replace'), out / 'trace.jsonl', MEASURED)
            text = stats.read_text() if stats.exists() else ''
            quads = re.search(r'chrome_divider_quads: (\d+)', text)
            report['chrome_divider_quads'] = int(quads.group(1)) if quads else None
            bands = re.search(r'chrome_bar_bands: (\d+)', text)
            report['chrome_bar_bands'] = int(bands.group(1)) if bands else None
            state = report['state-1']
            wanted = (True, 0, 0, 0.0) if args.classic else (False, 6, 6, 0.12)
            ok = (state['classic'], state['gap'] if not args.classic else state['right_divider'], state['radius'],
                  float(state['dim'])) == wanted
            ok = ok and (report['chrome_divider_quads'] == 0 if args.classic else (report['chrome_divider_quads'] or 0) > 0)
            # The bar is the mode line in every look (VM2-STATUS-BAR).
            ok = ok and state['status_bar'] == 1 and (report['chrome_bar_bands'] or 0) > 0
            # The pills keep the bar's text readable (WCAG AA).
            if not args.classic:
                report['pill_contrast'] = [round(contrast(*state['pill']), 2), round(contrast(*state['state_pill']), 2)]
                ok = ok and min(report['pill_contrast']) >= 4.5
            for name, (got, want) in checks.items():
                ok = ok and near(got, want)
            if themes:
                focuses = [report[f'state-{step}']['focus'] for step in ['3'] + themes]
                report['theme_focus'] = dict(zip(['none'] + themes, focuses))
                ok = ok and len(set(focuses)) == len(focuses)
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
