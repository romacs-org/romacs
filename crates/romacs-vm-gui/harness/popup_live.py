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
              :window-pixel-top (window-pixel-top window)
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
              :dashboard-top (window-pixel-top dashboard)
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--font', default='Iosevka')
    parser.add_argument('--font-size', default='14')
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
            (home / 'config/romacs/init.el').write_text(INIT.format(state=json.dumps(str(state))))
            (config / 'rc.xml').write_text(live.RC_XML)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': args.font, 'ROMACS_GUI_FONT_SIZE': args.font_size}
            for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_GATE_FONTCONFIG'):
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
                # the private compositor's socket, never the owner's display.
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
                    return [text for _, text, _ in rows(glyphs(), 'ModeLine')]

                fio.wait(lambda: text_rows(), args.timeout, 'first frame', process)
                keys('-M', 'alt', 'x', '-m', 'alt')
                fio.wait(lambda: ff.current_rows(log_path.read_text(errors='replace'))[:1] == ['M-x '], args.timeout, 'M-x prompt', process)
                keys('hermes', '-k', 'Return')
                fio.wait(lambda: any('*Hermes Dashboard*' in row for row in mode_lines()), args.timeout, 'dashboard', process)
                time.sleep(1)
                fio.shot(output / 'dashboard.png', env, report)
                report['captures'].append({'name': 'dashboard', 'rows': text_rows(), 'mode_lines': mode_lines()})
                # The dashboard opens its popup itself; the popup key shows
                # it again, refitting.
                fio.wait(state.exists, args.timeout, 'first popup fit', process)
                report['first_fit'] = state.read_text()
                state.unlink()
                keys('h')
                fio.wait(state.exists, args.timeout, 'popup fit', process)
                fio.wait(lambda: len(mode_lines()) >= 2 and any('Navigate' in row for row in text_rows()),
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
                # The popup is the bottom window: its band is the last mode line,
                # its text the rows between the band above and its band.
                bands = rows(painted, 'ModeLine')
                text = rows(painted, 'Text')
                line = probe['frame-char-height']
                top = probe['window-pixel-top']
                band_y = bands[-1][0]
                popup_rows = [r for r in text if top <= r[0] < band_y]
                report['checks'].append({'popup_rows': [t for _, t, _ in popup_rows], 'band_y': band_y, 'top': top})
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
                band = probe['window-pixel-height'] - probe['window-body-pixel-height']
                text_bottom = top + height - band
                if text_bottom > band_y:
                    raise AssertionError(f'text ends at {text_bottom}, below the mode line at {band_y}')
                # fit-window-to-buffer rounds to whole lines (pixelwise nil).
                if probe['window-pixel-height'] != -(-height // line) * line:
                    raise AssertionError(f"window {probe['window-pixel-height']}px for {height}px of text and band")
                if band_y - text_bottom >= line:
                    raise AssertionError(f'{band_y - text_bottom}px of blank rows above the mode line')
                widest = max(x for _, _, row in popup_rows for x, _ in row)
                report['checks'].append({'text_size': [width, height], 'painted_last_glyph_x': widest})
                # Each posn's X, Y is where the painter drew its glyph.
                for name, window_top in (('posn-first', top), ('posn-last', top), ('dashboard-posn', probe['dashboard-top'])):
                    posn = probe[name]
                    if not isinstance(posn, list):
                        raise AssertionError(f'{name}: {posn}')
                    _, (x, y), _, _ = posn
                    at = [g for g in painted if g['role'] == 'Text' and g['x'] == x and g['y'] == window_top + y]
                    report['checks'].append({name: posn, 'painted': [g['char'] for g in at]})
                    if name != 'dashboard-posn' and not at:
                        raise AssertionError(f'{name} {posn}: no glyph painted at ({x}, {window_top + y})')
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
