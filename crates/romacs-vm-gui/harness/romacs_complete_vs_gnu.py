#!/usr/bin/env python3
"""romacs-complete against vertico in pinned GNU, on the same keys.

romacs_complete_compare.el runs a list of readers (completing-read over
a word list, with a default, history, orderless components, groups;
read-file-name in a fixture directory) with keys typed into each as a
keyboard macro, and records, after every command in the minibuffer, the
input, selected index, total, scroll and the rows shown, then the value
the reader returned.

GNU: the pinned X11 build ($ROMACS_GNU_GUI_BINARY) on a private Xvfb
($ROMACS_DISPLAY_TOOLS), `-Q', with fixtures/vertico-c60d802 (vertico,
vertico-sort with `vertico-sort-history-length-alpha', vertico-directory
on RET, DEL and M-DEL) and the orderless.el fixture, `vertico-count' 12.
Romacs: `--loadup' in a private headless labwc with `romacs-complete-mode'.
Both use the same fixture directory, so file names compare as they are.
The run passes when the two record lists are equal.

usage: romacs_complete_vs_gnu.py --binary BIN --output DIR
"""
import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import tempfile
import time

import live_oracle as live

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
DRIVER = HERE / 'romacs_complete_compare.el'
VERTICO = HERE / 'fixtures' / 'vertico-c60d802'
ORDERLESS = REPO / 'crates' / 'romacs-vm' / 'tests' / 'fixtures' / 'orderless-e271a8e' / 'orderless.el'
GNU_SETUP = '''(progn
  (add-to-list 'load-path {vertico})
  (load {orderless} nil t)
  (require 'vertico) (require 'vertico-sort) (require 'vertico-directory)
  (setq vertico-count 12 vertico-sort-function #'vertico-sort-history-length-alpha)
  (keymap-set vertico-map "RET" #'vertico-directory-enter)
  (keymap-set vertico-map "DEL" #'vertico-directory-delete-char)
  (keymap-set vertico-map "M-DEL" #'vertico-directory-delete-word)
  (vertico-mode 1)
  (load {driver} nil t))'''


def fixture(directory):
    """The directory the file readers complete in."""
    (directory / 'sub' / 'nested').mkdir(parents=True)
    (directory / 'sub' / 'notes.txt').write_text('notes\n')
    (directory / 'sub' / 'nothing.md').write_text('x\n')
    for n in range(40):
        (directory / f'file-{n:02}.txt').write_text('x\n')
    for name in ('file.o', 'file.c', 'File.md', 'alpha.el', 'Alpine.md', 'beta.txt', '.hidden'):
        (directory / name).write_text('x\n')


def free_display():
    for number in range(140, 200):
        with socket.socket() as s:
            if s.connect_ex(('127.0.0.1', 6000 + number)) != 0:
                return number
    raise SystemExit('no free X display')


def run_gnu(root, directory, out, timeout):
    tools = Path(os.environ['ROMACS_DISPLAY_TOOLS'])
    number = free_display()
    display = f'127.0.0.1:{number}'
    auth = root / 'Xauthority'
    subprocess.run([tools / 'xauth', '-f', auth, 'add', display, 'MIT-MAGIC-COOKIE-1', secrets.token_hex(16)], check=True)
    server = subprocess.Popen([tools / 'Xvfb', f':{number}', '-screen', '0', '1280x800x24', '-nolisten', 'unix',
                               '-listen', 'tcp', '-nolock', '-noreset', '-auth', auth],
                              cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(50):
            with socket.socket() as s:
                if s.connect_ex(('127.0.0.1', 6000 + number)) == 0:
                    break
            time.sleep(0.1)
        home = root / 'gnu-home'
        home.mkdir()
        env = {'PATH': os.environ['PATH'], 'HOME': str(home), 'DISPLAY': display, 'XAUTHORITY': str(auth),
               'GDK_BACKEND': 'x11', 'LANG': 'C.UTF-8', 'CMP_DIR': str(directory), 'CMP_OUT': str(out),
               'TMPDIR': str(root)}
        setup = GNU_SETUP.format(vertico=json.dumps(str(VERTICO)), orderless=json.dumps(str(ORDERLESS)),
                                 driver=json.dumps(str(DRIVER)))
        subprocess.run([os.environ['ROMACS_GNU_GUI_BINARY'], '-Q', '--eval', setup], env=env, cwd=directory,
                       check=True, timeout=timeout, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    finally:
        server.terminate()
        server.wait()


def run_romacs(binary, root, directory, out, log_path, timeout):
    home, config = root / 'home', root / 'config'
    (home / 'config' / 'romacs').mkdir(parents=True)
    config.mkdir()
    (config / 'rc.xml').write_text(live.RC_XML)
    (home / 'config' / 'romacs' / 'init.el').write_text(
        f'(romacs-complete-mode 1)\n(load {json.dumps(str(DRIVER))} nil t)\n')
    env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
           'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
           'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
           'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
           'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
           'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
           'CMP_DIR': str(directory), 'CMP_OUT': str(out)}
    for key in ('CARGO_HOME', 'ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR', 'ROMACS_WORLD'):
        if key in os.environ:
            env[key] = os.environ[key]
    compositor = process = None
    with (root / 'compositor.log').open('w') as clog, log_path.open('w') as log:
        try:
            compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                          env=env, stdout=clog, stderr=clog)
            deadline = time.monotonic() + 15
            while not any(p.is_socket() for p in root.glob('wayland-*')):
                if time.monotonic() > deadline:
                    raise RuntimeError('no compositor socket')
                time.sleep(.1)
            env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
            process = subprocess.Popen([str(binary), '--loadup'], env=env, cwd=directory,
                                       stdin=subprocess.DEVNULL, stdout=log, stderr=log)
            process.wait(timeout)
        finally:
            live.stop(process)
            live.stop(compositor)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', type=float, default=600)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(args.binary.resolve()), 'status': 'FAIL'}
    try:
        with tempfile.TemporaryDirectory(prefix='rcvg-', dir=os.environ.get('XDG_RUNTIME_DIR')) as tmp:
            root = Path(tmp)
            root.chmod(0o700)
            directory = root / 'work'
            directory.mkdir()
            fixture(directory)
            (root / 'gnu').mkdir()
            (root / 'romacs').mkdir()
            run_gnu(root / 'gnu', directory, output / 'gnu.records', args.timeout)
            run_romacs(args.binary.resolve(), root / 'romacs', directory, output / 'romacs.records',
                       output / 'romacs.stderr', args.timeout)
            gnu = (output / 'gnu.records').read_text().replace(str(root), '$ROOT').splitlines()
            romacs = (output / 'romacs.records').read_text().replace(str(root), '$ROOT').splitlines()
            (output / 'gnu.records').write_text('\n'.join(gnu) + '\n')
            (output / 'romacs.records').write_text('\n'.join(romacs) + '\n')
            differ = [{'gnu': g, 'romacs': r} for g, r in zip(gnu, romacs) if g != r]
            report.update(records=len(gnu), romacs_records=len(romacs), differ=differ[:20],
                          scenarios=sorted({line.split()[0].lstrip('(') for line in gnu}))
            report['status'] = 'PASS' if gnu and gnu == romacs else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(json.dumps({k: report[k] for k in report if k != 'differ'}, ensure_ascii=False))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
