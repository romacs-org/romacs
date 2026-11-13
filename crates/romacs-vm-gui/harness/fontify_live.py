#!/usr/bin/env python3
"""A visited Lisp file fontified by redisplay, in a native --loadup session.

The session launches in a private headless compositor with no font override,
visiting a file of GNU's subr.el lines (its header, then its code).  Nothing asks for fontification:
a private init.el only records any call of `font-lock-ensure' or
`font-lock-fontify-buffer'.  The first frame must show the file in colour,
from jit-lock run by redisplay through `fontification-functions': the
submitted scene's `(defmacro' and comment glyphs differ in foreground from
plain text.  Real keys then run M-x romacs-live-fontify-state, which records
the buffer's `fontified' extent: fontified from the window start, not to the
end of the file.  labwc, wlr-randr, wtype and grim come from PATH (the gate's
headless tools).
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live

# The visited file fills the frame, as after a GNU init that inhibits the
# startup screen; without it GNU, and --loadup, show the screen's concise
# form beside the file.
INIT = live.NO_LINE_NUMBERS + '''(setq inhibit-startup-screen t)
(dolist (name '(font-lock-ensure font-lock-fontify-buffer))
  (advice-add name :before
              (lambda (&rest _) (write-region (format "%s\\n" name) nil {explicit} t))))
(defun romacs-live-fontify-state ()
  "Record how much of the visited file redisplay fontified."
  (interactive)
  (with-current-buffer (get-file-buffer {sample})
    (write-region
     (prin1-to-string
      (list major-mode (and (memq 'jit-lock-function fontification-functions) t)
            (get-text-property 1 'fontified)
            (next-single-property-change 1 'fontified)
            (point-max)
            (save-excursion (goto-char 1) (search-forward "(defmacro")
                            (get-text-property (- (point) 8) 'face))))
     nil {state})))
'''

LINES = 400


def scene_glyphs(text):
    """The last submitted scene's Text glyphs as (y, x, char, foreground)."""
    scene = fio.last_scene(text) or {}
    return sorted((g['y'], g['x'], g['char'], g['foreground'])
                  for g in scene.get('glyphs', []) if g['role'] == 'Text')


def row_colours(glyphs, prefix):
    """Foregrounds of the first scene row starting with PREFIX, by column."""
    rows = {}
    for y, _, char, foreground in glyphs:
        rows.setdefault(y, []).append((char, foreground))
    for row in rows.values():
        if ''.join(c for c, _ in row).startswith(prefix):
            return row
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    subr = Path(__file__).resolve().parents[3] / 'lisp/gnu' / 'subr.el'
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': []}
    paths = {}

    def prepare(s):
        paths.update(explicit=s.work / 'explicit-calls', state=s.work / 'fontify-state.el', sample=s.work / 'sample.el')
        lines = subr.read_text().splitlines(keepends=True)
        # Its header comment, then its code from the first definition's comment.
        paths['sample'].write_text(''.join(lines[:1] + lines[26:LINES]))
        (s.home / 'config/romacs/init.el').write_text(INIT.format(**{name: json.dumps(str(path)) for name, path in paths.items()}))

    with live.session(binary, output, prefix='fontify-live-', args=('--loadup', '--trace-scene', 'sample.el'), prepare=prepare, report=report) as live_session:
        process, env, log_path, root, work, home = (live_session.process, live_session.env, live_session.log, live_session.root,
                                                   live_session.work, live_session.home)
        explicit, state, sample = paths['explicit'], paths['state'], paths['sample']
        # Only the private compositor: no X display, no launch font.
        assert not any(key in env for key in ('DISPLAY', 'ROMACS_GUI_FONT', 'ROMACS_GUI_FONT_SIZE')), env
        socket = root / env['WAYLAND_DISPLAY']
        report['display'] = {'WAYLAND_DISPLAY': env['WAYLAND_DISPLAY'], 'socket': str(socket),
                             'socket_in_private_root': socket.is_socket()}

        def text():
            return log_path.read_text(errors='replace')

        def rows():
            return ff.current_rows(text())

        fio.wait(lambda: any(row.startswith('(defmacro declare-function') for row in rows()),
                 args.timeout, 'sample.el shown', process)
        time.sleep(1)
        fio.shot(output / 'fontified.png', env, report)
        glyphs = scene_glyphs(text())
        report['captures'].append({'name': 'fontified', 'rows': rows()})
        defmacro = row_colours(glyphs, '(defmacro declare-function')
        comment = row_colours(glyphs, ';;; subr.el')
        plain = defmacro and defmacro[0][1]
        colours = {'paren': plain, 'defmacro': sorted({f for _, f in defmacro[1:9]}) if defmacro else None,
                   'comment': sorted({f for _, f in comment}) if comment else None,
                   'distinct_text_foregrounds': sorted({g[3] for g in glyphs})}
        report['checks'].append({'scene_colours': colours})
        if not defmacro or not comment or plain in colours['defmacro'] or plain in colours['comment']:
            raise AssertionError(f'not fontified on screen: {colours}')
        subprocess.run(['wtype', '-d', '40', '-M', 'alt', 'x', '-m', 'alt'], env=env, check=True, timeout=20)
        fio.wait(lambda: ff.prompt_shown(rows(), 'M-x '), args.timeout, 'M-x prompt', process)
        subprocess.run(['wtype', '-d', '40', 'romacs-live-fontify-state', '-k', 'Return'],
                       env=env, check=True, timeout=20)
        fio.wait(state.exists, args.timeout, 'fontify state', process)
        recorded = state.read_text()
        report['checks'].append({'fontify_state': recorded})
        mode, jit, first, end, size, face = recorded.strip('()').split(' ')
        if (mode, jit, first, face) != ('emacs-lisp-mode', 't', 't', 'font-lock-keyword-face') \
                or not int(end) < int(size):
            raise AssertionError(f'fontify state: {recorded}')
        calls = explicit.read_text() if explicit.exists() else ''
        report['checks'].append({'explicit_fontify_calls': calls})
        if calls:
            raise AssertionError(f'explicit fontification ran: {calls}')
        report['status'] = 'PASS'
    print(report['status'])
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
