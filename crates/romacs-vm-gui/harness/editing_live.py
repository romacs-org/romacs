#!/usr/bin/env python3
"""Native editing and shared terminal-region journey in a private compositor."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=20)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': []}
    compositor = keyboard = process = None
    try:
        with tempfile.TemporaryDirectory(prefix='editing-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config):
                directory.mkdir()
            original = work / 'editing.txt'
            initial = 'First λ line\nSecond line\n'
            original.write_text(initial)
            (config / 'rc.xml').write_text(live.RC_XML.replace('</labwc_config>',
                '<keyboard><keybind key="A-F4"><action name="Close"/></keybind></keyboard></labwc_config>'))
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '20', 'SHELL': '/usr/bin/bash'}
            log_path = output / 'binary.stderr'
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([live.LABWC, '-C', str(config)], env=env,
                                              stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), args.timeout,
                         'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([live.RANDR, '--output', 'HEADLESS-1', '--custom-mode', '2532x1347'],
                               env=env, check=True, timeout=15)
                keyboard = subprocess.Popen(['/usr/bin/wtype', '-s', '120000', '-k', 'Shift_L'],
                                            env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(binary), '--trace-scene', str(original)], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def trace():
                    return log_path.read_text(errors='replace')

                def keys(*parts):
                    subprocess.run(['/usr/bin/wtype', '-d', '40', *parts], env=env, check=True, timeout=20)
                    time.sleep(.15)

                def control(key):
                    if key == 'space':
                        keys('-M', 'ctrl', '-k', 'space', '-m', 'ctrl')
                    else:
                        keys('-M', 'ctrl', key, '-m', 'ctrl')

                def meta(key):
                    keys('-M', 'alt', key, '-m', 'alt')

                def visible(text):
                    fio.wait(lambda: any(text in row for row in ff.current_rows(trace())),
                             args.timeout, text, process)

                def capture(name):
                    fio.shot(output / (name + '.png'), env, report)
                    report['captures'].append({'name': name, 'rows': ff.current_rows(trace())})

                def extended(name):
                    meta('x')
                    visible('M-x ')
                    keys(name, '-k', 'Return')

                def register(action, name):
                    control('x')
                    keys('r')
                    keys(action)
                    keys(name)

                visible('First λ line')
                control('x')
                keys('1')
                meta(':')
                visible('Eval: ')
                keys('(+ 20 22)')
                capture('eval-expression-prompt')
                keys('-k', 'Return')
                visible('42')
                assert original.read_text() == initial
                capture('eval-expression-result')
                report['checks'].append('NATIVE_META_COLON_EVALUATES_WITHOUT_DOCUMENT_EDIT')
                meta('<')
                control('space')
                control('n')
                capture('ordinary-region-selected')
                register('s', 'a')
                meta('<')
                control('space')
                control('n')
                meta('w')
                meta('>')
                control('y')
                register('i', 'a')
                expected = initial + 'First λ line\n' * 2
                control('x')
                control('s')
                fio.wait(lambda: original.read_text() == expected, args.timeout, 'region and register exact save', process)
                capture('region-yank-register-saved')
                report['checks'].append('NATIVE_REGION_KILL_RING_AND_ONE_KEY_TEXT_REGISTER')
                extended('romacs-term')
                keys('stty -echo; printf "\\033[31mCOPY_ME\\033[0m\\nTERMINAL_READY\\n"', '-k', 'Return')
                visible('TERMINAL_READY')
                control('c')
                control('c')
                meta('<')
                control('n')
                control('a')
                control('s')
                visible('Search terminal:')
                keys('COPY_ME', '-k', 'Return')
                for _ in range(7):
                    control('b')
                control('space')
                for _ in range(7):
                    control('f')
                capture('terminal-ansi-shared-region')
                scene = fio.last_scene(trace())
                assert scene is not None, 'TERMINAL_SELECTION_SCENE_REQUIRED'
                selected = [g for g in scene['glyphs']
                            if g['role'] == 'Text' and g['background'] == '#3e4451']
                assert ''.join(g['char'] for g in selected) == 'COPY_ME', 'EXACT_TERMINAL_REGION_IS_PAINTED'
                assert all(g['foreground'] not in ('#d4c5a0', '#000000') for g in selected), 'ANSI_FOREGROUND_SURVIVES_REGION'
                meta('w')
                control('x')
                keys('b')
                visible('Switch to buffer')
                keys('editing.txt', '-k', 'Return')
                meta('>')
                control('y')
                expected += 'COPY_ME'
                control('x')
                control('s')
                fio.wait(lambda: original.read_text() == expected, args.timeout, 'terminal copy exact save', process)
                capture('terminal-region-yanked-into-file')
                report['checks'].append('NATIVE_TERMINAL_AND_EDITOR_SHARE_ORDINARY_REGION_AND_KILL_RING')
                keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
                visible('Running terminals exist;')
                keys('y')
                process.wait(timeout=args.timeout)
                assert process.returncode == 0
                report.update(status='PASS', exit=0, saved_text=original.read_text())
                return 0
    except Exception as error:
        report['error'] = repr(error)
        return 1
    finally:
        for child in (process, keyboard, compositor):
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        (output / 'INDEX.txt').write_text('\n'.join(c['name'] + '.png' for c in report['captures']) + '\n')


if __name__ == '__main__':
    raise SystemExit(main())
