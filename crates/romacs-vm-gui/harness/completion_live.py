#!/usr/bin/env python3
"""GNU minibuffer completion in a native --loadup session, in a private compositor.

The session launches with no font override, in a work directory holding
sub/notes.txt.  A private init.el records, when init runs, the completion
options Romacs's defaults gave it (rlisp/romacs-completion-defaults.el) and
defines a probe command that records when it runs.  Real keys then drive
GNU's readers through icomplete's fido-vertical display:

- M-x with a few letters shows the vertical candidate list in a grown
  mini-window, TAB force-completes the selected command, Return runs it;
- C-x C-f with partial completion of s/n to sub/notes.txt, TAB, Return
  visits the file;
- C-x b lists the buffers with the default in the prompt, TAB takes
  *scratch*, Return switches to it.

Each state is captured as a screenshot and as the prompt's minibuffer text.
labwc, wlr-randr, wtype and grim come from PATH (the gate's headless tools).
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

INIT = '''(write-region
 (prin1-to-string
  (list completion-styles completion-auto-select completion-auto-help completions-format
        completions-sort completions-max-height completion-ignore-case
        icomplete-compute-delay icomplete-delay-completions-threshold
        completion-category-overrides fido-vertical-mode
        (lookup-key icomplete-fido-mode-map (kbd "TAB"))))
 nil {defaults})
(defun romacs-live-completion-probe ()
  "Record that M-x ran this command."
  (interactive)
  (write-region "ran" nil {probe}))
'''

EXPECTED_DEFAULTS = ('((basic flex) t visible one-column historical 20 t 0.15 400 '
                     '((file (styles partial-completion))) t icomplete-force-complete)')


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
        with tempfile.TemporaryDirectory(prefix='completion-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work / 'sub', config):
                directory.mkdir(parents=True)
            (work / 'sub' / 'notes.txt').write_text('Notes reached by partial completion.\n')
            defaults, probe = work / 'defaults.el', work / 'probe'
            (home / 'config/romacs').mkdir(parents=True)
            (home / 'config/romacs/init.el').write_text(INIT.format(
                defaults=json.dumps(str(defaults)), probe=json.dumps(str(probe))))
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
                keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                # Only the private compositor: no X display, no launch font.
                assert not any(key in env for key in ('DISPLAY', 'ROMACS_GUI_FONT', 'ROMACS_GUI_FONT_SIZE')), env
                socket = root / env['WAYLAND_DISPLAY']
                report['display'] = {'WAYLAND_DISPLAY': env['WAYLAND_DISPLAY'], 'socket': str(socket),
                                     'socket_in_private_root': socket.is_socket()}
                process = subprocess.Popen([str(binary), '--loadup', '--trace-scene'], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def minibuffer():
                    rows = ff.current_rows(log_path.read_text(errors='replace'))
                    return rows[0] if len(rows) == 1 else ''

                def keys(*parts):
                    subprocess.run(['wtype', '-d', '60', *parts], env=env, check=True, timeout=20)

                def shown(name, what, test):
                    """Wait until the prompt shows TEST, then capture it as NAME."""
                    fio.wait(lambda: test(minibuffer()), args.timeout, what, process)
                    time.sleep(.5)
                    fio.shot(output / f'{name}.png', env, report)
                    report['captures'].append({'name': name, 'minibuffer': minibuffer()})

                def mode_lines():
                    scene = fio.last_scene(log_path.read_text(errors='replace'))
                    by_y = {}
                    for glyph in (scene or {}).get('glyphs', []):
                        if glyph['role'] == 'ModeLine':
                            by_y.setdefault(glyph['y'], []).append((glyph['x'], glyph['char']))
                    return [''.join(c for _, c in sorted(row)) for _, row in sorted(by_y.items())]

                def selected(name, buffer):
                    """Wait until a mode line names BUFFER, then capture it as NAME."""
                    fio.wait(lambda: any(buffer in row for row in mode_lines()), args.timeout, f'{buffer} shown', process)
                    time.sleep(.5)
                    fio.shot(output / f'{name}.png', env, report)
                    report['captures'].append({'name': name, 'mode_lines': mode_lines()})

                fio.wait(defaults.exists, args.timeout, 'init.el after --loadup', process)
                at_init = defaults.read_text()
                report['checks'].append({'defaults_at_init': at_init})
                if at_init != EXPECTED_DEFAULTS:
                    raise AssertionError(f'defaults at init: {at_init}, expected {EXPECTED_DEFAULTS}')
                fio.wait(lambda: ff.current_rows(log_path.read_text(errors='replace')), args.timeout, 'first frame', process)
                time.sleep(1)

                # M-x: the vertical list, TAB, Return.
                keys('-M', 'alt', 'x', '-m', 'alt')
                fio.wait(lambda: 'M-x' in minibuffer(), args.timeout, 'M-x prompt', process)
                keys('romacs-live-com')
                shown('mx-list', 'M-x candidates',
                      lambda m: 'M-x romacs-live-com' in m and '\nromacs-live-completion-probe' in m)
                keys('-k', 'Tab')
                shown('mx-tab', 'M-x force-completed', lambda m: 'M-x romacs-live-completion-probe ' in m)
                keys('-k', 'Return')
                fio.wait(probe.exists, args.timeout, 'the probe command ran', process)
                report['checks'].append({'mx_ran': probe.read_text()})

                # C-x C-f: partial completion of s/n.
                keys('-M', 'ctrl', 'x', '-m', 'ctrl', '-M', 'ctrl', 'f', '-m', 'ctrl')
                fio.wait(lambda: 'Find file: ' in minibuffer(), args.timeout, 'find-file prompt', process)
                keys('s/n')
                shown('ff-list', 'partial completion of s/n',
                      lambda m: f'Find file: {work}/s/n' in m and '\nsub/notes.txt' in m)
                keys('-k', 'Tab')
                shown('ff-tab', 'find-file force-completed', lambda m: f'Find file: {work}/sub/notes.txt' in m)
                keys('-k', 'Return')
                selected('ff-visited', 'notes.txt')

                # C-x b: buffers, the default in the prompt.
                keys('-M', 'ctrl', 'x', '-m', 'ctrl', 'b')
                fio.wait(lambda: 'Switch to buffer (default ' in minibuffer(), args.timeout, 'switch prompt', process)
                keys('scr')
                shown('cxb-list', 'buffer candidates', lambda m: '): scr' in m and '\n*scratch*' in m)
                keys('-k', 'Tab')
                shown('cxb-tab', 'buffer force-completed', lambda m: '): *scratch*' in m)
                keys('-k', 'Return')
                selected('cxb-switched', '*scratch*')
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
