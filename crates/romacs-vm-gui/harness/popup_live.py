#!/usr/bin/env python3
"""A hermes keymap popup sized by fit-window-to-buffer, in a private compositor.

A native --loadup session with a private HOME and init.el (hermes pointed at
a loopback port nothing listens on) runs M-x hermes, then the dashboard's
popup key (the dashboard opens the popup itself first; that one is
dismissed with q).  keymap-popup displays its buffer in a bottom side window and
fits it with `fit-window-to-buffer', which measures with
`window-text-pixel-size' in the window the host has laid out.  An advice
around the fit records, in the popup's window, what the fit asked and got
and `posn-at-point' at the popup's first and last positions, and in the
dashboard's window `posn-at-point' at point.

The judge is the painted scene, not those answers: the popup window's
painted text rows must be exactly its buffer's lines, each row one line
high, with its mode line directly below the last, and each recorded posn's
pixel position must be where the painter drew that glyph.  labwc,
wlr-randr, wtype and grim come from PATH (the gate's headless tools).

With --child-frame, keymap-popup shows its buffer in a child frame
(`keymap-popup-backend-child-frame', fitted by `fit-frame-to-buffer'), a
layer over the frame: the popup's painted rows must be exactly its buffer's
lines, in the child frame's rect, and the root frame's windows and mode
lines must not move while it shows or after it goes.

With --which-key, which-key's `custom' popup type shows its keys in a
layer (romacs-layer.el) after C-x: a strip directly above the minibuffer
(the frame's bottom edge while romacs-complete's minibuffer layer is
hidden), with the root frame's windows unmoved while it shows and after it
goes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live

INIT = '''(setq hermes-dashboard-transport-url "http://127.0.0.1:9"
      hermes-dashboard-transport-start-mode 'remote
      hermes-dashboard-transport-command "/nonexistent/hermes")
(defun popup-live--posn (window pos)
  (let ((posn (posn-at-point pos window)))
    (and posn (list (posn-point posn) (posn-x-y posn) (posn-actual-col-row posn)
                    (posn-object-width-height posn)))))
(defun popup-live--record (fit buffer)
  (let* ((before (get-buffer-window buffer))
         (old (and before (window-pixel-height before)))
         (outcome (condition-case error (progn (funcall fit buffer) 'fitted)
                    (error (list 'error (error-message-string error)))))
         (window (get-buffer-window buffer))
         (dashboard (get-buffer-window "*Hermes Dashboard*")))
    (with-current-buffer buffer
      (write-region
       (prin1-to-string
        (list :outcome outcome :height-before old
              :lines (count-lines (point-min) (point-max))
              :window-pixel-height (window-pixel-height window)
              :window-body-pixel-height (window-body-height window t)
              :mode-line-height (window-mode-line-height window)
              ;; Its top as painted: past the frame's internal border.
              :window-pixel-top (nth 1 (window-edges window nil nil t))
              :text-left (car (window-inside-pixel-edges window))
              :frame-char-height (frame-char-height)
              :text-size (condition-case error (window-text-pixel-size window nil t nil nil t)
                           (error (error-message-string error)))
              :buffer-text-size (condition-case error (buffer-text-pixel-size buffer window)
                                  (error (error-message-string error)))
              :posn-first (condition-case error (popup-live--posn window (point-min))
                            (error (error-message-string error)))
              :posn-last (condition-case error
                             (popup-live--posn window (save-excursion (goto-char (point-max)) (skip-chars-backward "\\n") (1- (point))))
                           (error (error-message-string error)))
              :dashboard-posn (condition-case error (popup-live--posn dashboard (window-point dashboard))
                                (error (error-message-string error)))
              :dashboard-top (nth 1 (window-edges dashboard nil nil t))
              :dashboard-text-left (car (window-inside-pixel-edges dashboard))
              :line-starts (condition-case error
                               (save-excursion
                                 (goto-char (point-min))
                                 (let (out)
                                   (while (not (eobp))
                                     (push (list (buffer-substring-no-properties (point) (line-end-position))
                                                 (cdr (posn-x-y (posn-at-point (point) window))))
                                           out)
                                     (forward-line 1))
                                   (nreverse out)))
                             (error (error-message-string error)))))
       nil {state}))
    (if (eq outcome 'fitted) outcome (error "%s" (cadr outcome)))))
(with-eval-after-load 'keymap-popup
  (advice-add 'keymap-popup--fit-side-window :around #'popup-live--record))
'''


CHILD_INIT = '''(setq hermes-dashboard-transport-url "http://127.0.0.1:9"
      hermes-dashboard-transport-start-mode 'remote
      hermes-dashboard-transport-command "/nonexistent/hermes")
(setq keymap-popup-backend #'keymap-popup-backend-child-frame)
(defun popup-live--root-windows ()
  (mapcar (lambda (w) (list (buffer-name (window-buffer w)) (window-pixel-top w) (window-pixel-height w)))
          (window-list (frame-parent (selected-frame)) 'nomini)))
(defun popup-live--record (buffer)
  (let* ((frame (keymap-popup--session-get buffer :frame))
         (window (frame-root-window frame)))
    (with-current-buffer buffer
      (write-region
       (prin1-to-string
        (list :position (list (car (frame-position frame)) (cdr (frame-position frame)))
              :native (list (frame-native-width frame) (frame-native-height frame))
              :border (frame-parameter frame 'child-frame-border-width)
              :visible (frame-visible-p frame)
              :window-pixel-top (window-pixel-top window)
              :window-pixel-left (window-pixel-left window)
              :text-left (car (window-inside-pixel-edges window))
              :frame-char-height (frame-char-height)
              :mode-line (and (buffer-local-value 'mode-line-format buffer) t)
              :lines (save-excursion
                       (goto-char (point-min))
                       (let (out)
                         (while (not (eobp))
                           (push (buffer-substring-no-properties (point) (line-end-position)) out)
                           (forward-line 1))
                         (nreverse out)))
              :root-windows (popup-live--root-windows)
              :frames (length (frame-list))))
       nil {state} nil (quote quiet)))))
(with-eval-after-load 'keymap-popup
  (advice-add 'keymap-popup--show-child-frame :after #'popup-live--record))
'''


WHICH_KEY_INIT = '''(require 'which-key)
(setq which-key-popup-type 'custom
      which-key-custom-show-popup-function #'romacs-layer-which-key-show
      which-key-custom-hide-popup-function #'romacs-layer-which-key-hide
      which-key-custom-popup-max-dimensions-function #'romacs-layer-which-key-max-dimensions
      which-key-idle-delay 0.3)
(which-key-mode 1)
(defun popup-live--record (&rest _)
  (let ((frame (cdr (assq which-key--buffer romacs-layer--frames))))
    (write-region
     (prin1-to-string
      (list :position (list (car (frame-position frame)) (cdr (frame-position frame)))
            :native (list (frame-native-width frame) (frame-native-height frame))
            ;; Where the minibuffer begins: its line, its layer while that
            ;; shows, else (its layer hidden) the frame's bottom edge.
            :minibuffer-top (let ((host (romacs--minibuffer-host)))
                              (cond ((null host) (window-pixel-top (minibuffer-window)))
                                    ((frame-visible-p host) (cdr (frame-position host)))
                                    (t (frame-text-height (frame-parent frame)))))
            ;; Where a bottom layer ends: above the status bar.
            :chrome-bottom (romacs-chrome-bottom (frame-parent frame) frame)
            ;; Child frames are placed outside the root's internal border.
            :inset (frame-internal-border-width (frame-parent frame))
            :root-windows (mapcar (lambda (w) (list (window-pixel-top w) (window-pixel-height w)))
                                  (window-list (frame-parent frame) 'nomini))))
     nil {state} nil (quote quiet))))
(advice-add 'romacs-layer-which-key-show :after #'popup-live--record)
'''


def check_which_key(args, output, report, env, process, state, keys, glyphs):
    """C-x shows which-key's keys in a layer above the status bar and the
    minibuffer (the bar sits on the minibuffer line, or the frame's bottom
    while the minibuffer's layer is hidden); the root frame's windows do
    not move."""
    def root_bands():
        return [y for y, _, _ in rows([g for g in glyphs() if g.get('frame', 1) == 1], 'StatusBar') + rows([g for g in glyphs() if g.get('frame', 1) == 1], 'ModeLine')]

    def layer_rows():
        return rows([g for g in glyphs() if g.get('frame', 1) != 1], 'Text')

    # The frame takes the output's size after its first paint: wait for the
    # bar to stay where it is before comparing.
    def settled():
        first = root_bands()
        time.sleep(.3)
        return bool(first) and first == root_bands()
    fio.wait(settled, args.timeout, 'root bar settled', process)
    before = root_bands()
    keys('-M', 'ctrl', 'x', '-m', 'ctrl')
    fio.wait(state.exists, args.timeout, 'which-key layer', process)
    fio.wait(lambda: layer_rows(), args.timeout, 'which-key painted', process)
    time.sleep(1)
    fio.shot(output / 'which-key.png', env, report)
    probe = read_plist(state.read_text())
    report['probe'] = probe
    shown = [text for _, text, _ in layer_rows()]
    report['captures'].append({'name': 'which-key', 'layer_rows': shown, 'root_bands': root_bands()})
    if not any('b : switch-to-buffer' in row for row in shown):
        raise AssertionError(f'which-key layer shows {shown}')
    left, top = probe['position']
    width, height = probe['native']
    if (left, top + height) != (probe['inset'], probe['chrome-bottom']) or probe['chrome-bottom'] > probe['minibuffer-top']:
        raise AssertionError(f"layer at {left},{top} {width}x{height}, chrome bottom {probe['chrome-bottom']}, minibuffer line at {probe['minibuffer-top']}")
    if root_bands() != before:
        raise AssertionError(f'root mode lines at {root_bands()} with the layer, {before} without')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    fio.wait(lambda: not layer_rows(), args.timeout, 'which-key gone', process)
    time.sleep(.5)
    if root_bands() != before:
        raise AssertionError(f'root mode lines at {root_bands()} after the layer, {before} before')
    # The layer's frame is kept and shows again.
    keys('-M', 'ctrl', 'x', '-m', 'ctrl')
    fio.wait(lambda: any('b : switch-to-buffer' in text for _, text, _ in layer_rows()), args.timeout, 'which-key again', process)
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    fio.wait(lambda: not layer_rows(), args.timeout, 'which-key gone again', process)
    report['checks'].append({'layer_rect': [left, top, width, height], 'rows': len(shown), 'root_bands': before, 'shown_twice': True})


def scene_glyphs(text):
    scene = fio.last_scene(text)
    return (scene or {}).get('glyphs', [])


def rows(glyphs, role):
    """ROLE's painted rows, top down: (y, text, [(x, char)])."""
    by_y = {}
    for glyph in glyphs:
        if glyph['role'] == role:
            by_y.setdefault(glyph['y'], []).append((glyph['x'], glyph['char']))
    return [(y, ''.join(c for _, c in sorted(row)), sorted(row)) for y, row in sorted(by_y.items())]


def read_plist(text):
    """The few Lisp shapes the init writes: lists, integers, strings, symbols."""
    tokens, i = [], 0
    while i < len(text):
        c = text[i]
        if c in '()':
            tokens.append(c); i += 1
        elif c.isspace():
            i += 1
        elif c == '"':
            j, out = i + 1, ''
            while text[j] != '"':
                if text[j] == '\\':
                    j += 1
                out += text[j]; j += 1
            tokens.append(('str', out)); i = j + 1
        else:
            j = i
            while j < len(text) and not text[j].isspace() and text[j] not in '()':
                j += 1
            tokens.append(('atom', text[i:j])); i = j

    def parse(at):
        token = tokens[at]
        if token == '(':
            items, at = [], at + 1
            while tokens[at] != ')':
                item, at = parse(at)
                items.append(item)
            # A dotted pair (A . B) reads as [A, B].
            return [x for x in items if x != '.'], at + 1
        kind, value = token
        if kind == 'str':
            return value, at + 1
        try:
            return int(value), at + 1
        except ValueError:
            return (None if value == 'nil' else value), at + 1

    value, _ = parse(0)
    return {key.lstrip(':'): item for key, item in zip(value[::2], value[1::2])}


def check_child_frame(args, output, report, env, process, state, keys, glyphs, log_path):
    """The popup is a child frame: its rows are its buffer's lines in its
    rect, painted over the root frame, whose windows never move."""
    def root(role):
        return rows([g for g in glyphs() if g.get('frame', 1) == 1], role)

    fio.wait(state.exists, args.timeout, 'first popup', process)
    state.unlink()
    keys('h')
    fio.wait(state.exists, args.timeout, 'popup shown', process)
    probe = read_plist(state.read_text())
    report['probe'] = probe

    def popup_rows():
        return rows([g for g in glyphs() if g.get('frame', 1) != 1], 'Text')

    fio.wait(lambda: any(text.strip() for _, text, _ in popup_rows()), args.timeout, 'popup painted', process)
    time.sleep(1.5)
    fio.shot(output / 'popup.png', env, report)
    # The root frame's mode line: its status bar (VM2-STATUS-BAR).
    shown = {'root_mode_lines': [(y, t) for y, t, _ in root('StatusBar') + root('ModeLine')], 'popup_rows': [(y, t) for y, t, _ in popup_rows()]}
    report['captures'].append({'name': 'popup', **shown})
    lines = [line.rstrip() for line in probe['lines'] if line.strip()]
    # A line as wide as the window continues on the next row, as GNU
    # shows it without fringes (the last column holds the `\\').
    painted, carry = [], ''
    for _, text in shown['popup_rows']:
        if text.endswith('\\'):
            carry += text[:-1]
            continue
        painted.append((carry + text).rstrip())
        carry = ''
    painted = [text for text in painted if text.strip()]
    if painted != lines:
        raise AssertionError(f'popup painted {painted} for its lines {lines}')
    # Its rows start at its window's top inside the frame's rect, a line apart.
    left, top = probe['position']
    first_y = shown['popup_rows'][0][0]
    line = probe['frame-char-height']
    if first_y != top + probe['window-pixel-top']:
        raise AssertionError(f"first popup row at {first_y}, frame top {top} + window top {probe['window-pixel-top']}")
    ys = [y for y, _ in shown['popup_rows']]
    if any(not line <= b - a <= 2 * line + 1 for a, b in zip(ys, ys[1:])):
        raise AssertionError(f'popup rows at {ys}, not a line ({line}px) apart')
    width, height = probe['native']
    xs = [x for _, _, row in popup_rows() for x, _ in row]
    if min(xs) < left + probe['text-left'] or max(xs) >= left + width:
        raise AssertionError(f'popup glyphs span {min(xs)}..{max(xs)}, frame at {left} width {width}')
    if ys[-1] + line > top + height:
        raise AssertionError(f'last popup row {ys[-1]} below the frame bottom {top + height}')
    # The root frame's windows are as they were with the popup gone.
    keys('q')
    fio.wait(lambda: not popup_rows(), args.timeout, 'popup gone', process)
    time.sleep(1)
    fio.shot(output / 'gone.png', env, report)
    gone = [(y, t) for y, t, _ in root('StatusBar') + root('ModeLine')]
    report['captures'].append({'name': 'gone', 'root_mode_lines': gone})
    if [y for y, _ in gone] != [y for y, _ in shown['root_mode_lines']]:
        raise AssertionError(f"root mode lines at {[y for y, _ in gone]} after, {[y for y, _ in shown['root_mode_lines']]} while the popup showed")
    report['checks'].append({'popup_lines': len(lines), 'popup_rect': [left, top, width, height],
                             'root_windows': probe['root-windows'], 'root_mode_line_y': [y for y, _ in gone]})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--font', default='Iosevka')
    parser.add_argument('--font-size', default='14')
    parser.add_argument('--child-frame', action='store_true', help="keymap-popup's child-frame backend")
    parser.add_argument('--which-key', action='store_true', help="which-key's popup in a layer")
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': []}
    compositor = keyboard = process = None
    try:
        with tempfile.TemporaryDirectory(prefix='popup-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config):
                directory.mkdir()
            state = work / 'popup-state.el'
            (home / 'config/romacs').mkdir(parents=True)
            init = WHICH_KEY_INIT if args.which_key else CHILD_INIT if args.child_frame else INIT
            (home / 'config/romacs/init.el').write_text(init.format(state=json.dumps(str(state))))
            (config / 'rc.xml').write_text(live.RC_XML)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': args.font, 'ROMACS_GUI_FONT_SIZE': args.font_size}
            for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_WORLD', 'ROMACS_GATE_FONTCONFIG'):
                if key in os.environ:
                    env[key] = os.environ[key]
            if 'ROMACS_GATE_FONTCONFIG' in env:
                env['FONTCONFIG_FILE'] = env['ROMACS_GATE_FONTCONFIG']
            log_path = output / 'binary.stderr'
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                # Every client below connects to XDG_RUNTIME_DIR/WAYLAND_DISPLAY:
                # the private compositor's socket, never the user's display.
                socket = Path(env['XDG_RUNTIME_DIR']) / env['WAYLAND_DISPLAY']
                assert socket.is_socket() and socket.parent == root, socket
                assert 'DISPLAY' not in env
                report['wayland_socket'] = str(socket)
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)
                keyboard = subprocess.Popen(['wtype', '-s', '120000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(binary), '--loadup', '--trace-scene'], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def glyphs():
                    return scene_glyphs(log_path.read_text(errors='replace'))

                def keys(*parts):
                    subprocess.run(['wtype', '-d', '40', *parts], env=env, check=True, timeout=20)
                    time.sleep(.15)

                def text_rows():
                    return [text for _, text, _ in rows(glyphs(), 'Text')]

                def mode_lines():
                    # The status bar is the mode line (VM2-STATUS-BAR), with
                    # the popup's own line folded into it.
                    return [text for _, text, _ in rows(glyphs(), 'StatusBar') + rows(glyphs(), 'ModeLine')]

                fio.wait(lambda: text_rows(), args.timeout, 'first frame', process)
                if args.which_key:
                    time.sleep(1)
                    check_which_key(args, output, report, env, process, state, keys, glyphs)
                    report['status'] = 'PASS'
                    return
                keys('-M', 'alt', 'x', '-m', 'alt')
                fio.wait(lambda: ff.prompt_shown(ff.current_rows(log_path.read_text(errors='replace')), 'M-x '), args.timeout, 'M-x prompt', process)
                keys('hermes', '-k', 'Return')
                fio.wait(lambda: any('*Hermes Dashboard*' in row for row in mode_lines()), args.timeout, 'dashboard', process)
                time.sleep(1)
                fio.shot(output / 'dashboard.png', env, report)
                report['captures'].append({'name': 'dashboard', 'rows': text_rows(), 'mode_lines': mode_lines()})
                if args.child_frame:
                    check_child_frame(args, output, report, env, process, state, keys, glyphs, log_path)
                    report['status'] = 'PASS'
                    return
                # The dashboard opens its popup itself; the popup key shows
                # it again, refitting.
                fio.wait(state.exists, args.timeout, 'first popup fit', process)
                report['first_fit'] = state.read_text()
                state.unlink()
                keys('h')
                fio.wait(state.exists, args.timeout, 'popup fit', process)
                fio.wait(lambda: mode_lines() and any('Navigate' in row for row in text_rows()),
                         args.timeout, 'popup painted', process)
                time.sleep(1.5)
                fio.shot(output / 'popup.png', env, report)
                painted = glyphs()
                report['captures'].append({'name': 'popup', 'rows': text_rows(), 'mode_lines': mode_lines()})
                probe = read_plist(state.read_text())
                report['probe'] = probe
                report['probe_source'] = state.read_text()
                if probe['outcome'] != 'fitted':
                    raise AssertionError(f"fit-window-to-buffer: {probe['outcome']}")
                # The popup is the bottom window: its text is the rows between
                # its top and its body's bottom edge.  A GUI window has no
                # mode-line row (its line folds into the status bar), so the
                # edge comes from the window's geometry, not a painted band.
                text = rows(painted, 'Text')
                line = probe['frame-char-height']
                top = probe['window-pixel-top']
                # The text-size counts the window's mode line (none under
                # the status bar); the body excludes it and the divider.
                band = probe['mode-line-height']
                edge = top + probe['window-body-pixel-height']
                popup_rows = [r for r in text if top <= r[0] < edge]
                report['checks'].append({'popup_rows': [t for _, t, _ in popup_rows], 'edge': edge, 'top': top})
                # Each line's painted row is where posn-at-point puts it,
                # every nonblank line is painted, and nothing is cut.
                starts = probe['line-starts']
                if not isinstance(starts, list):
                    raise AssertionError(f'line starts: {starts}')
                painted_at = {y: text for y, text, _ in popup_rows}
                for line_text, y in starts:
                    if line_text.strip() and painted_at.get(top + y, '').rstrip() != line_text.rstrip():
                        raise AssertionError(f'line {line_text!r} at {top + y}: painted {painted_at.get(top + y)!r}')
                if len([t for t, _ in starts if t.strip()]) != len(popup_rows):
                    raise AssertionError(f'{len(popup_rows)} painted rows for lines {starts}')
                width, height = probe['text-size']
                text_bottom = top + height - band
                if text_bottom > edge:
                    raise AssertionError(f'text ends at {text_bottom}, below the body edge at {edge}')
                # fit-window-to-buffer rounds to whole lines (pixelwise nil),
                # fitting the text and what the window has besides its body
                # (its bottom divider under the pane chrome), as GNU's does.
                needed = height - band + probe['window-pixel-height'] - probe['window-body-pixel-height']
                if probe['window-pixel-height'] != -(-needed // line) * line:
                    raise AssertionError(f"window {probe['window-pixel-height']}px for {height}px of text and {needed - height}px besides")
                if edge - text_bottom >= line:
                    raise AssertionError(f'{edge - text_bottom}px of blank rows above the body edge')
                widest = max(x for _, _, row in popup_rows for x, _ in row)
                report['checks'].append({'text_size': [width, height], 'painted_last_glyph_x': widest})
                # Each posn's X, Y is where the painter drew its glyph: X and
                # Y count from the window's text area, which starts after
                # its left margin (keymap-popup's buffer has one) and
                # includes its header line (posn-at-x-y's doc).
                for name, window_top, text_left in (('posn-first', top, probe['text-left']),
                                                    ('posn-last', top, probe['text-left']),
                                                    ('dashboard-posn', probe['dashboard-top'], probe['dashboard-text-left'])):
                    posn = probe[name]
                    if not isinstance(posn, list):
                        raise AssertionError(f'{name}: {posn}')
                    _, (x, y), _, _ = posn
                    at = [g for g in painted if g['role'] == 'Text' and g['x'] == text_left + x and g['y'] == window_top + y]
                    report['checks'].append({name: posn, 'text_left': text_left, 'painted': [g['char'] for g in at]})
                    if name != 'dashboard-posn' and not at:
                        raise AssertionError(f'{name} {posn}: no glyph painted at ({text_left + x}, {window_top + y})')
                report['status'] = 'PASS'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        live.stop(process)
        live.stop(keyboard)
        live.stop(compositor)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'])
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
