#!/usr/bin/env python3
"""Bundled package autoloads in a native --loadup session, in a private compositor.

A private init.el records, when init runs, whether `hermes' and
`gnosis-dashboard' are still unloaded autoloads; then real keys run
M-x gnosis-dashboard, which must load the package and show its buffer.
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
  (mapcar (lambda (name)
            (list name (autoloadp (symbol-function name)) (commandp name) (featurep name)))
          '(hermes gnosis-dashboard)))
 nil {state})
'''


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
            state = work / 'init-state.el'
            (home / 'config/romacs').mkdir(parents=True)
            (home / 'config/romacs/init.el').write_text(INIT.format(state=json.dumps(str(state))))
            (config / 'rc.xml').write_text(live.RC_XML)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '14'}
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
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)
                # A held key gives the seat a keyboard before the client starts.
                keyboard = subprocess.Popen(['wtype', '-s', '120000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(binary), '--loadup', '--trace-scene'], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def rows():
                    return ff.current_rows(log_path.read_text(errors='replace'))

                def keys(*parts):
                    subprocess.run(['wtype', '-d', '40', *parts], env=env, check=True, timeout=20)
                    time.sleep(.15)

                def visible(text):
                    fio.wait(lambda: any(text in row for row in rows()), args.timeout, text, process)

                fio.wait(state.exists, args.timeout, 'init.el after --loadup', process)
                at_init = state.read_text()
                report['checks'].append({'init_state': at_init})
                expected = '((hermes t t nil) (gnosis-dashboard t t nil))'
                if at_init != expected:
                    raise AssertionError(f'at init: {at_init}, expected {expected}')
                fio.wait(rows, args.timeout, 'first frame', process)
                fio.shot(output / 'started.png', env, report)
                report['captures'].append({'name': 'started', 'rows': rows()})
                keys('-M', 'alt', 'x', '-m', 'alt')
                fio.wait(lambda: rows()[:1] == ['M-x '], args.timeout, 'M-x prompt', process)
                keys('gnosis-dashboard', '-k', 'Return')
                try:
                    visible('*Gnosis Dashboard*')
                finally:
                    report['captures'].append({'name': 'after-command', 'rows': rows()})
                    fio.shot(output / 'after-command.png', env, report)
                report['checks'].append({'gnosis_dashboard_visible': True})
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
