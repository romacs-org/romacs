#!/usr/bin/env python3
"""Record pinned GNU's `format-mode-line' answers on a terminal frame.

GNU formats nothing in batch, so this runs `emacs -Q -nw' over a pty with a
private HOME, evaluates SETUP and then each case in the selected window, and
writes format-mode-line-tty.txt beside this script: SETUP, then each case
and GNU's `prin1' of its answer (an error as (signal ERR)).

usage: format-mode-line-tty.py EMACS
"""
import os
from pathlib import Path
import pty
import select
import struct
import sys
import tempfile
import time
import fcntl
import termios

SETUP = ('(progn (switch-to-buffer (get-buffer-create "fml")) (insert "line one\\nline two\\n") (goto-char 5)'
         ' (defvar fml-on nil) (defvar fml-name "lit%b") (setq mode-name "Fml"))')
CASES = [
    '(format-mode-line "%b")',
    '(format-mode-line "%b %l:%c %C")',
    '(format-mode-line \'("" "a" (:propertize "b" face bold) "c"))',
    '(format-mode-line \'("" "a" (:propertize "b" face bold) "c") \'italic)',
    '(format-mode-line \'(:propertize "%b" face bold) t)',
    '(format-mode-line \'(:propertize "%b" face bold) 1)',
    '(format-mode-line \'("" "x" (:propertize "y" face bold)) \'default)',
    '(format-mode-line "%5b|")',
    '(format-mode-line \'(-2 "abcdef"))',
    '(format-mode-line \'(8 "ab") \'bold)',
    '(format-mode-line \'("" (6 "ab") "|"))',
    '(format-mode-line "%-")',
    '(format-mode-line "<%3->")',
    '(format-mode-line \'(:eval (concat "x" "y")))',
    '(format-mode-line \'(fml-on "yes" "no"))',
    '(format-mode-line \'("" fml-name))',
    '(format-mode-line "a\\tb")',
    '(format-mode-line "%*%+%&")',
    '(format-mode-line "é%b")',
    '(format-mode-line \'("" mode-name))',
    '(format-mode-line \'("" "%b" (:eval (format-mode-line "-%l-"))))',
    '(format-mode-line "%p %P %q %i %n")',
    '(format-mode-line \'("" "%b") nil nil (get-buffer "*scratch*"))',
    '(format-mode-line "%%%m")',
    # Two windows on one buffer at different points: WINDOW is selected
    # while it formats, and the selection and point come back after.
    '''(let ((w (split-window))) (set-window-point w 14) (prog1 (list (format-mode-line "%l:%c" nil w) (format-mode-line '(:eval (format "%d" (point))) nil w) (format-mode-line "%l:%c") (point) (eq (selected-window) (frame-first-window)) (buffer-name) (window-point w)) (delete-window w)))''',
]


def main():
    emacs = sys.argv[1]
    here = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix='fml-tty-') as temporary:
        home = Path(temporary)
        out = home / 'answers.el'
        cases = home / 'cases.el'
        cases.write_text(''.join(f'{case}\n' for case in CASES))
        program = (f'(progn {SETUP}'
                   f' (with-temp-buffer (insert-file-contents {str(cases)!r}) (let ((print-escape-newlines t) (answers nil))'
                   ' (while (not (eobp)) (let ((form (read (current-buffer)))) (skip-chars-forward "\\n")'
                   ' (push (prin1-to-string (with-current-buffer "fml" (condition-case e (eval form t) (error (list (quote signal) e))))) answers)))'
                   f' (write-region (mapconcat (function identity) (nreverse answers) "\\n") nil {str(out)!r})))'
                   ' (kill-emacs 0))').replace("'", '"')
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'TERM': 'xterm-256color', 'LANG': 'C.UTF-8',
               'XDG_CONFIG_HOME': str(home / 'c'), 'XDG_CACHE_HOME': str(home / 'k'), 'XDG_DATA_HOME': str(home / 'd')}
        pid, fd = pty.fork()
        if pid == 0:
            os.execve(emacs, [emacs, '-Q', '-nw', '--eval', f'(run-with-timer 0.5 nil (lambda () {program}))'], env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if select.select([fd], [], [], 0.1)[0]:
                try:
                    os.read(fd, 65536)
                except OSError:
                    break
            done, status = os.waitpid(pid, os.WNOHANG)
            if done:
                break
        else:
            os.kill(pid, 9)
            raise SystemExit('GNU did not finish')
        answers = out.read_text().split('\n')
    assert len(answers) == len(CASES), answers
    lines = [';; Pinned GNU on an 80x24 terminal frame, by format-mode-line-tty.py; do not edit.', SETUP]
    for case, answer in zip(CASES, answers):
        lines += [case, answer]
    (here / 'format-mode-line-tty.txt').write_text('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
