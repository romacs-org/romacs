#!/usr/bin/env python3
"""Record pinned GNU's startup order after the init file, on a terminal.

GNU runs no startup hooks in batch (`command-line' ends in `kill-emacs'),
so this runs `emacs -nw --no-site-file' over a pty with a private HOME
whose ~/.emacs.d/init.el is each scenario's INIT, optionally with a file
argument, and writes startup-order-tty.txt beside this script: per
scenario its name, whether a file is visited, INIT, and GNU's `prin1' of
`startup-order-log' as `window-setup-hook' leaves it.  startup.rs's tests
evaluate the same INIT where the GUI loads the user's init.

usage: startup-order-tty.py EMACS
"""
import fcntl
import os
from pathlib import Path
import pty
import select
import struct
import sys
import tempfile
import termios
import time

# Each init logs (HOOK BUFFER-SHOWN) at the startup hooks; the last one
# also records *scratch*.  The file argument is named visited.txt.
LOG = ('(defvar startup-order-log nil)'
       ' (defun startup-order-note (hook) (push (list hook (buffer-name (window-buffer (selected-window)))) startup-order-log))'
       ' (add-hook (quote emacs-startup-hook) (lambda () (startup-order-note (quote emacs-startup))))'
       ' (add-hook (quote window-setup-hook) (lambda () (startup-order-note (quote window-setup))'
       ' (push (with-current-buffer "*scratch*" (list (buffer-substring-no-properties (point-min) (point-max)) (buffer-modified-p) major-mode)) startup-order-log)'
       ' (setq startup-order-log (nreverse startup-order-log))))')
SCENARIOS = [
    # An after-init hook that switches buffers: the file argument is
    # visited after it, and the later hooks see the file.
    ('hook-then-file', True,
     LOG + ' (add-hook (quote after-init-hook) (lambda () (startup-order-note (quote after-init))'
           ' (switch-to-buffer (get-buffer-create "*hooked*"))))'),
    # An init that inhibits the startup screen and writes *scratch* keeps it.
    ('own-scratch', False,
     LOG + ' (setq inhibit-startup-screen t)'
           ' (with-current-buffer "*scratch*" (erase-buffer) (insert "mine\\n"))'
           ' (add-hook (quote after-init-hook) (lambda () (startup-order-note (quote after-init))))'),
]


def record(emacs, init, with_file):
    with tempfile.TemporaryDirectory(prefix='startup-order-') as temporary:
        home = Path(temporary)
        (home / '.emacs.d').mkdir()
        out = home / 'answer.el'
        (home / '.emacs.d/init.el').write_text(
            init + f' (add-hook (quote window-setup-hook) (lambda () (with-temp-file {str(out)!r} (let ((print-escape-newlines t)) (prin1 startup-order-log (current-buffer)))) (kill-emacs 0)) 90)\n'
            .replace("'", '"'))
        (home / 'visited.txt').write_text('visited\n')
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'TERM': 'xterm-256color', 'LANG': 'C.UTF-8',
               'XDG_CONFIG_HOME': str(home / 'c'), 'XDG_CACHE_HOME': str(home / 'k'), 'XDG_DATA_HOME': str(home / 'd')}
        args = [emacs, '-nw', '--no-site-file'] + (['visited.txt'] if with_file else [])
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(home)
            os.execve(emacs, args, env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if select.select([fd], [], [], 0.1)[0]:
                try:
                    os.read(fd, 65536)
                except OSError:
                    break
            done, _ = os.waitpid(pid, os.WNOHANG)
            if done:
                break
        else:
            os.kill(pid, 9)
            raise SystemExit('GNU did not finish')
        return out.read_text()


def main():
    emacs = sys.argv[1]
    lines = [';; Pinned GNU on an 80x24 terminal, by startup-order-tty.py; do not edit.']
    for name, with_file, init in SCENARIOS:
        lines += [name, 't' if with_file else 'nil', init, record(emacs, init, with_file)]
    (Path(__file__).resolve().parent / 'startup-order-tty.txt').write_text('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
