#!/usr/bin/env python3
"""Native editing and shared terminal-region journey in a private compositor."""
import argparse
import hashlib
from pathlib import Path
import subprocess
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live


def exercise_selection_replacement(process, keys, control, meta, capture, original, timeout, report):
    def save(expected, label):
        control('x')
        control('s')
        fio.wait(lambda: original.read_text() == expected, timeout, label, process)
        capture(label)
        report['checks'].append(label)

    meta('<')
    control('space')
    control('n')
    capture('backspace-selection')
    keys('-k', 'BackSpace')
    save('Second line\n', 'NATIVE_BACKSPACE_DELETES_SELECTED_REGION_ONLY')
    meta('<')
    control('space')
    control('n')
    keys('Replacement λ')
    save('Replacement λ', 'NATIVE_TYPING_REPLACES_SELECTED_REGION')
    meta('>')
    control('space')
    meta('<')
    capture('reverse-delete-selection')
    control('d')
    save('', 'NATIVE_C_D_DELETES_REVERSED_REGION_ONLY')
    keys('Editor still usable')
    save('Editor still usable', 'NATIVE_EDITING_CONTINUES_AFTER_SELECTION_DELETION')
    keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
    process.wait(timeout=timeout)
    assert process.returncode == 0
    report.update(status='PASS', exit=0, saved_text=original.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=20)
    parser.add_argument('--delete-selection', action='store_true')
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': []}
    initial = 'First λ line\nSecond line\n'
    original = None

    def prepare(s):
        nonlocal original
        original = s.work / 'editing.txt'
        original.write_text(initial)

    rc_xml = live.RC_XML.replace('</labwc_config>',
        '<keyboard><keybind key="A-F4"><action name="Close"/></keybind></keyboard></labwc_config>')
    try:
        with live.session(binary, output, prefix='editing-live-', args=('--trace-scene', 'editing.txt'), rc_xml=rc_xml,
                          env={'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '20', 'SHELL': '/usr/bin/bash'},
                          size='2532x1347', prepare=prepare, report=report) as s:
            env, process, log_path = s.env, s.process, s.log

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
            if args.delete_selection:
                exercise_selection_replacement(process, keys, control, meta, capture,
                                               original, args.timeout, report)
                return 0
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
    finally:
        (output / 'INDEX.txt').write_text('\n'.join(c['name'] + '.png' for c in report['captures']) + '\n')
    return 1


if __name__ == '__main__':
    raise SystemExit(main())
