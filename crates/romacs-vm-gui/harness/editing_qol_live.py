#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Private native clipboard, word-delete and disposable mail-draft GUI journey.

Use an already-built binary: --binary PATH --output NEW_DIRECTORY. All input,
clipboard fixtures and captures target a new headless compositor, never the
user's display. wl-copy/wl-paste are external fixture applications only; native
C-y/M-w must traverse the editor's ordinary GNU kill/selection hooks. No mail
index, account configuration, send command or transport is used.

Submitted trace rows synchronize input; retained PNGs require human visual
review. This is correctness acceptance, not a performance benchmark.
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


CLIPBOARD = 'Clipboard λ — café\n第二行: naïve e\u0301\n'.encode('utf-8')
ORDINARY = b'Ordinary file remains intact\nsecond line\n'
WORDS = b'one_two three-four'
WORD_PREFIX = b'one_two three-'
DRAFT_BODY = 'Private unsent fixture λ; never send this draft.'
TOOLS = '/nix/store/klrgwbvpwd7napzdhacw1nhj8xmpbpdc-romacs-gate-headless-tools/bin'
QUESTIONS = (
    'Do we provide the user all the essential information properly formatted?',
    'Do we reduce noise/clutter?',
    'Is this pretty and easy on the eyes?',
)


def private_env(root: Path) -> dict:
    """Allowlist the child environment, omitting every ambient display/socket."""
    home = root / 'home'
    home.mkdir()
    env = {'PATH': TOOLS + ':/usr/bin:/bin', 'HOME': str(home), 'LANG': 'C.UTF-8',
           'XDG_RUNTIME_DIR': str(root), 'TMPDIR': str(root),
           'XDG_CONFIG_DIRS': str(home / 'config'), 'XDG_DATA_DIRS': '/usr/local/share:/usr/share',
           'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
           'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
           'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '20', 'SHELL': '/usr/bin/bash'}
    for suffix in ('CONFIG', 'CACHE', 'DATA', 'STATE'):
        directory = home / suffix.lower()
        directory.mkdir()
        env['XDG_' + suffix + '_HOME'] = str(directory)
    return env


def executable(value: str, path: str) -> str:
    resolved = shutil.which(value, path=path)
    if resolved is None:
        raise ValueError('executable not found: ' + value)
    return str(Path(resolved).resolve())


def run(args, output: Path, report: dict) -> None:
    # Keep short socket paths and all temporary writes under owned scratch.
    scratch = Path(os.environ.get('TMPDIR', str(Path.home() / '.cache' / 'romacs-tests'))).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    if len(os.fsencode(str(scratch))) + 32 >= 108:
        raise ValueError('TMPDIR is too long for a private Wayland socket')
    with tempfile.TemporaryDirectory(prefix='eq-', dir=scratch) as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        env = private_env(root)
        init = Path(env['XDG_CONFIG_HOME']) / 'romacs' / 'init.el'
        init.parent.mkdir()
        init.write_text('(setq romacs-qol-init-proof "PRIVATE_INIT_LOADED")\n', encoding='utf-8')
        tools = {name: executable(name, env['PATH'])
                 for name in ('wtype', 'wl-copy', 'wl-paste', 'grim')}
        labwc = executable(live.LABWC, env['PATH'])
        randr = executable(live.RANDR, env['PATH'])
        work, config = root / 'work', root / 'compositor'
        work.mkdir()
        config.mkdir()
        (config / 'rc.xml').write_text(live.RC_XML.replace('</labwc_config>',
            '<keyboard><keybind key="A-F4"><action name="Close"/></keybind></keyboard></labwc_config>'),
            encoding='utf-8')
        ordinary = work / 'ordinary.txt'
        ordinary.write_bytes(ORDINARY)
        pasted = work / 'clipboard.txt'
        pasted.write_bytes(b'')
        for name in ('control-word.txt', 'meta-word.txt'):
            (work / name).write_bytes(WORDS)
        compositor = keyboard = process = owner = None
        log_path = output / 'binary.stderr'
        try:
            with (output / 'compositor.log').open('w') as compositor_log, \
                    log_path.open('w') as log, (output / 'clipboard-fixture.log').open('w') as clipboard_log:
                compositor = subprocess.Popen([labwc, '-C', str(config)], env=env, cwd=work,
                                              stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')),
                         args.timeout, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                report['private_display'] = {'runtime': str(root), 'socket': env['WAYLAND_DISPLAY']}
                subprocess.run([randr, '--output', 'HEADLESS-1', '--custom-mode', '2532x1347'],
                               env=env, check=True, timeout=15, stdout=compositor_log, stderr=compositor_log)
                keyboard = subprocess.Popen([tools['wtype'], '-s', '120000', '-k', 'Shift_L'],
                                            env=env, cwd=work, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(args.binary), '--trace-scene', str(ordinary)],
                                           env=env, cwd=work, stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def trace():
                    return log_path.read_text(encoding='utf-8', errors='replace')

                def visible(text):
                    fio.wait(lambda: any(text in row for row in ff.current_rows(trace())),
                             args.timeout, text, process)

                def keys(*parts):
                    subprocess.run([tools['wtype'], '-d', '10', *parts], env=env,
                                   check=True, timeout=30, stdout=log, stderr=log)
                    time.sleep(.15)

                def control(key):
                    keys('-M', 'ctrl', '-k', key, '-m', 'ctrl')

                def meta(key):
                    keys('-M', 'alt', key, '-m', 'alt')

                def evaluate(expression, marker):
                    # Require a new marker after Return, not text in the input prompt.
                    meta(':')
                    visible('Eval: ')
                    keys('(progn ' + expression + ' (message "' + marker + '"))')
                    start = len(trace())
                    keys('-k', 'Return')
                    fio.wait(lambda: any(row == marker or row == '"' + marker + '"'
                                         for row in ff.current_rows(trace()[start:])),
                             args.timeout, marker, process)

                def require(predicate, marker):
                    evaluate('(unless ' + predicate + ' (error "QOL assertion failed"))', marker)
                    report['checks'].append(marker)

                def find(name):
                    control('x')
                    control('f')
                    visible('Find file:')
                    keys(name, '-k', 'Return')
                    fio.wait(lambda: not any('Find file:' in row for row in ff.current_rows(trace())),
                             args.timeout, 'file visit completed', process)

                def save(path, expected, label):
                    control('x')
                    control('s')
                    fio.wait(lambda: path.read_bytes() == expected, args.timeout, label, process)
                    actual = path.read_bytes()
                    (output / (label + '.txt')).write_bytes(actual)
                    report['saved_bytes'][label] = actual.hex()
                    report['checks'].append(label)

                def capture(name):
                    path = output / (name + '.png')
                    fio.shot(path, env, report)
                    if not path.is_file():
                        raise AssertionError('required compositor capture missing: ' + name)
                    report['captures'].append({'name': name, 'rows': ff.current_rows(trace())})

                def clipboard_bytes():
                    result = subprocess.run([tools['wl-paste'], '--no-newline', '--type', 'text/plain'],
                                            env=env, check=False, capture_output=True, timeout=5)
                    if result.returncode:
                        # A just-started wl-copy has not necessarily published
                        # its offer yet. Poll absence, but retain its diagnostic.
                        report['clipboard_last_error'] = result.stderr.decode('utf-8', errors='replace')
                        return None
                    return result.stdout

                def fixture_clipboard(data):
                    nonlocal owner
                    live.stop(owner)
                    # Foreground mode lets cleanup reap the fixture, rather than
                    # leaving wl-copy's normal daemon behind on the private socket.
                    owner = subprocess.Popen([tools['wl-copy'], '--foreground', '--type',
                                              'text/plain;charset=utf-8'], env=env, cwd=work,
                                             stdin=subprocess.PIPE, stdout=clipboard_log, stderr=clipboard_log)
                    if owner.stdin is None:
                        raise RuntimeError('clipboard fixture stdin pipe missing')
                    try:
                        owner.stdin.write(data)
                        owner.stdin.close()
                        fio.wait(lambda: clipboard_bytes() == data, args.timeout,
                                 'external private clipboard fixture exact bytes', owner)
                    finally:
                        if owner.stdin is not None and not owner.stdin.closed:
                            owner.stdin.close()

                visible('Ordinary file remains intact')
                require('(equal romacs-qol-init-proof "PRIVATE_INIT_LOADED")', 'QOL_PRIVATE_INIT_LOADED')
                control('x')
                keys('1')
                find('clipboard.txt')
                evaluate('(setq kill-ring nil kill-ring-yank-pointer nil)', 'QOL_EMPTY_RING')
                fixture_clipboard(CLIPBOARD)
                control('y')
                save(pasted, CLIPBOARD, 'external-unicode-multiline-paste')
                require('(and (= (length kill-ring) 1) (equal (car kill-ring) (buffer-string)))',
                        'QOL_EXTERNAL_PASTE_IMPORTED_ONCE')
                meta('<')
                control('space')
                meta('>')
                fixture_clipboard(b'M-w must replace this private external decoy')
                meta('w')
                # Matching the original fixture alone would not prove export.
                # Require both decoy-owner cancellation and exact native bytes.
                if owner is None:
                    raise AssertionError('external clipboard fixture missing')
                owner.wait(timeout=args.timeout)
                if owner.returncode != 0:
                    raise AssertionError('external clipboard owner failed: ' + str(owner.returncode))
                fio.wait(lambda: clipboard_bytes() == CLIPBOARD, args.timeout,
                         'native M-w exports exact clipboard bytes', process)
                exported = clipboard_bytes()
                if exported is None or exported != CLIPBOARD:
                    raise AssertionError('native clipboard bytes changed after export')
                (output / 'exported-clipboard.txt').write_bytes(exported)
                report['checks'].append('NATIVE_M_W_EXPORTS_EXACT_UNICODE_MULTILINE')
                evaluate('(setq qol-ring-length (length kill-ring) qol-ring-head (car kill-ring))',
                         'QOL_COPY_RING_PINNED')
                control('y')
                control('y')
                save(pasted, CLIPBOARD * 3, 'own-copy-repeated-yank')
                require('(and (> qol-ring-length 0) (= qol-ring-length (length kill-ring)) '
                        '(equal qol-ring-head (car kill-ring)))', 'QOL_OWN_COPY_DOES_NOT_GROW_RING')
                capture('clipboard-roundtrip')

                for name, modifier in (('control-word.txt', 'ctrl'), ('meta-word.txt', 'alt')):
                    find(name)
                    evaluate('(setq kill-ring nil kill-ring-yank-pointer nil)',
                             'QOL_RESET_' + modifier.upper())
                    fixture_clipboard(b'external word fixture; not the killed word')
                    meta('>')
                    keys('-M', modifier, '-k', 'BackSpace', '-m', modifier)
                    save(work / name, WORD_PREFIX, modifier + '-backspace-killed-four')
                    require('(and (= (length kill-ring) 1) (equal (car kill-ring) "four"))',
                            'QOL_' + modifier.upper() + '_BACKSPACE_IS_WORD_KILL')
                    fio.wait(lambda: clipboard_bytes() == b'four', args.timeout,
                             'word kill exported four', process)
                    control('y')
                    save(work / name, WORDS, modifier + '-backspace-yank-restores-four')
                capture('word-kill-restored')

                find('ordinary.txt')
                evaluate('(romacs-mail-compose \'(:name "fixture" :from "user@example.invalid"))',
                         'QOL_COMPOSE_OPENED')
                visible('--text follows this line--')
                keys(DRAFT_BODY)
                visible(DRAFT_BODY)
                evaluate('(setq qol-draft (current-buffer))', 'QOL_DRAFT_PINNED')
                require("(and (eq major-mode 'romacs-mail-compose-mode) "
                        '(buffer-modified-p) (not buffer-file-name))', 'QOL_REAL_DIRTY_MAIL_DRAFT')
                capture('dirty-mail-draft')
                control('x')
                keys('k')
                fio.wait(lambda: not any(DRAFT_BODY in row for row in ff.current_rows(trace())),
                         args.timeout, 'dirty mail draft killed without discard prompt or veto', process)
                require('(not (buffer-live-p qol-draft))', 'QOL_DIRTY_DRAFT_KILLED_WITHOUT_VETO')
                find('ordinary.txt')
                visible('Ordinary file remains intact')
                require('(equal (buffer-string) "Ordinary file remains intact\\nsecond line\\n")',
                        'QOL_ORDINARY_BUFFER_UNCHANGED')
                if ordinary.read_bytes() != ORDINARY:
                    raise AssertionError('ordinary file changed on disk')
                (output / 'ordinary-unchanged.txt').write_bytes(ordinary.read_bytes())
                report['saved_bytes']['ordinary-unchanged'] = ordinary.read_bytes().hex()
                capture('draft-killed-ordinary-intact')
                control('x')
                control('c')
                process.wait(timeout=args.timeout)
                report['native_exit'] = process.returncode
                if process.returncode != 0:
                    raise AssertionError('native exit was not zero: ' + str(process.returncode))
                report['status'] = 'PASS'
        except Exception:
            if process is not None and process.poll() is None:
                fio.shot(output / 'failure.png', env, report)
            raise
        finally:
            report['native_exit_before_cleanup'] = None if process is None else process.poll()
            for label, child in (('binary', process), ('clipboard_fixture', owner),
                                 ('keyboard', keyboard), ('compositor', compositor)):
                if child is not None:
                    induced = child.poll() is None
                    live.stop(child)
                    report['cleanup'][label] = {'exit': child.returncode, 'induced': induced}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', type=float, default=20)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error('--timeout must be positive')
    args.binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'status': 'FAIL', 'binary': str(args.binary), 'checks': [], 'captures': [],
              'saved_bytes': {}, 'cleanup': {}, 'geometry': [2532, 1347],
              'visual_review': {question: 'PENDING: inspect retained PNGs' for question in QUESTIONS},
              'scope': 'private native correctness; no performance claim or mail transport'}
    try:
        if not args.binary.is_file() or not os.access(args.binary, os.X_OK):
            raise ValueError('--binary must name an executable file')
        report['binary_sha256'] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
        run(args, output, report)
    except (OSError, ValueError, AssertionError, RuntimeError, subprocess.SubprocessError) as error:
        report['error'] = repr(error)
    finally:
        (output / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n',
                                            encoding='utf-8')
        (output / 'INDEX.txt').write_text('\n'.join(c['name'] + '.png' for c in report['captures']) + '\n',
                                          encoding='utf-8')
    print(json.dumps({'status': report['status'], 'native_exit': report.get('native_exit'),
                      'error': report.get('error'), 'output': str(output)}, ensure_ascii=False))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
