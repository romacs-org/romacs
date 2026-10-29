#!/usr/bin/env python3
"""Bundled package autoloads in a native --loadup session, in a private compositor.

The session launches with no font override, visiting an Emacs Lisp file.  A
private init.el records, when init runs, whether `hermes' and
`gnosis-dashboard' are still unloaded autoloads and the interactive
session's runtime-dependent settings, and points hermes at a loopback port
nothing listens on, attaching rather than spawning.  The default face must
render at a readable size.  Real keys then run M-x romacs-live-elisp-state,
which records the file's buffer as visited and after an explicit
`font-lock-ensure', M-x gnosis-dashboard, which must load the package and
show its buffer, and M-x hermes, which must load hermes and show its
dashboard without reaching any service.  The echo area after each command
is recorded.  labwc, wlr-randr, wtype and grim come from PATH (the gate's
headless tools).
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live

INIT = '''(write-region
 (prin1-to-string
  (mapcar (lambda (name)
            (list name (autoloadp (symbol-function name)) (commandp name) (featurep name)))
          '(hermes gnosis-dashboard)))
 nil {state})
(write-region
 (prin1-to-string
  (list global-font-lock-mode (expand-file-name abbrev-file-name)
        auto-save-list-file-prefix user-emacs-directory))
 nil {session})
(defun romacs-live-elisp-state ()
  "Record the visited Lisp file's buffer, then fontify it explicitly."
  (interactive)
  (with-current-buffer (get-file-buffer {sample})
    (let ((faces (lambda ()
                   (let ((pos (point-min)) seen)
                     (while (setq pos (next-single-property-change pos 'face))
                       (let ((face (get-text-property pos 'face)))
                         (unless (member face seen) (push face seen))))
                     (nreverse seen)))))
      (let ((visited (funcall faces)))
        (font-lock-ensure)
        (write-region
         (prin1-to-string
          (list major-mode font-lock-mode
                (and (memq 'jit-lock-function fontification-functions) t)
                visited (funcall faces)))
         nil {elisp})))))
(setq hermes-dashboard-transport-url "http://127.0.0.1:9"
      hermes-dashboard-transport-start-mode 'remote
      hermes-dashboard-transport-command "/nonexistent/hermes")
'''


SAMPLE = ''';;; sample.el --- A visited Lisp file  -*- lexical-binding: t -*-
;; A comment line.
(defun sample-greet (name)
  "Greet NAME."
  (message "Hello, %s" name))
'''


def scene_rows(text, role):
    """Rows of the last submitted scene's glyphs in ROLE (Text or ModeLine)."""
    scene = fio.last_scene(text)
    by_y = {}
    for glyph in (scene or {}).get('glyphs', []):
        if glyph['role'] == role:
            by_y.setdefault(glyph['y'], []).append((glyph['x'], glyph['char']))
    return [''.join(c for _, c in sorted(row)) for _, row in sorted(by_y.items())]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': []}
    compositor = keyboard = process = None
    try:
        with tempfile.TemporaryDirectory(prefix='autoloads-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config):
                directory.mkdir()
            state, session, elisp = work / 'init-state.el', work / 'session-state.el', work / 'elisp-state.el'
            sample = work / 'sample.el'
            sample.write_text(SAMPLE)
            (home / 'config/romacs').mkdir(parents=True)
            (home / 'config/romacs/init.el').write_text(INIT.format(
                **{name: json.dumps(str(path)) for name, path in
                   (('state', state), ('session', session), ('elisp', elisp), ('sample', sample))}))
            (config / 'rc.xml').write_text(live.RC_XML)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
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
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)
                # A held key gives the seat a keyboard before the client starts.
                keyboard = subprocess.Popen(['wtype', '-s', '120000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                # Only the private compositor: no X display, no launch font.
                assert not any(key in env for key in ('DISPLAY', 'ROMACS_GUI_FONT', 'ROMACS_GUI_FONT_SIZE')), env
                report['display'] = {'WAYLAND_DISPLAY': env['WAYLAND_DISPLAY'], 'socket_in_private_root':
                                     (root / env['WAYLAND_DISPLAY']).is_socket()}
                process = subprocess.Popen([str(binary), '--loadup', '--trace-scene', sample.name], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def rows():
                    return ff.current_rows(log_path.read_text(errors='replace'))

                def keys(*parts):
                    subprocess.run(['wtype', '-d', '40', *parts], env=env, check=True, timeout=20)
                    time.sleep(.15)

                def mode_lines():
                    return scene_rows(log_path.read_text(errors='replace'), 'ModeLine')

                def shown(buffer, text):
                    """BUFFER names a window's mode line and TEXT is in a window."""
                    fio.wait(lambda: any(buffer in row for row in mode_lines()) and any(text in row for row in rows()),
                             args.timeout, f'{buffer} showing {text}', process)

                def run(command, buffer, text):
                    keys('-M', 'alt', 'x', '-m', 'alt')
                    fio.wait(lambda: rows()[:1] and rows()[0].startswith('M-x '), args.timeout, 'M-x prompt', process)
                    keys(command, '-k', 'Return')
                    try:
                        shown(buffer, text)
                    finally:
                        time.sleep(1)
                        report['captures'].append({'name': command, 'rows': rows(), 'mode_lines': mode_lines()})
                        fio.shot(output / f'{command}.png', env, report)
                    echo = scene_rows(log_path.read_text(errors='replace'), 'Text')[-1:]
                    report['checks'].append({'command': command, 'shown': buffer, 'echo': echo})

                fio.wait(state.exists, args.timeout, 'init.el after --loadup', process)
                at_init = state.read_text()
                report['checks'].append({'init_state': at_init})
                expected = '((hermes t t nil) (gnosis-dashboard t t nil))'
                if at_init != expected:
                    raise AssertionError(f'at init: {at_init}, expected {expected}')
                at_init = session.read_text()
                report['checks'].append({'session_state': at_init})
                config_dir = f'{home}/config/romacs/'
                expected = f'(t "{config_dir}abbrev_defs" "{config_dir}auto-save-list/.saves-" "{config_dir}")'
                if at_init != expected:
                    raise AssertionError(f'session at init: {at_init}, expected {expected}')
                fio.wait(rows, args.timeout, 'first frame', process)
                fio.shot(output / 'started.png', env, report)
                report['captures'].append({'name': 'started', 'rows': rows()})
                # The launch font: no override, so the frame's own default.
                font = re.findall(r'^romacs: font family=(.*) pixels=(\d+)$', log_path.read_text(errors='replace'), re.M)
                report['checks'].append({'default_font': font})
                if not font or int(font[-1][1]) < 10:
                    raise AssertionError(f'default face font {font}: not a readable size')
                run('romacs-live-elisp-state', 'sample.el', '(defun sample-greet')
                fontified = elisp.read_text()
                report['checks'].append({'elisp_state': fontified})
                if not fontified.startswith('(emacs-lisp-mode t t ') or 'font-lock-keyword-face' not in fontified:
                    raise AssertionError(f'Lisp buffer: {fontified}')
                run('gnosis-dashboard', '*Gnosis Dashboard*', 'Gnosis Dashboard')
                run('hermes', '*Hermes Dashboard*', 'Hermes')
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
