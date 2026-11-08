#!/usr/bin/env python3
"""GNU's package.el and package-vc.el in a real session, in a private headless
labwc, HOME and XDG (server_live.py), over the network to real archives.

  M-x package-install RET magit RET with real keys, in a clean HOME: the prompt
  closes at once, keys go on reaching the buffer and the session goes on answering
  while a worker VM installs, and the session says "Magit installed" when it ends.
  The package works: `require', a command, no `.elc' files.  So does
  `package-vc-install' from a git URL.

usage: packages_live.py --romacs BIN --output DIR [--tools DIR]
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

import parity_live
from server_live import Server, until

ARCHIVES = '''(setq package-archives '(("gnu" . "https://elpa.gnu.org/packages/")
                                ("nongnu" . "https://elpa.nongnu.org/nongnu/")))'''
PACKAGE = 'magit'
GIT_URL = 'https://github.com/minad/marginalia'
GIT_PACKAGE = 'marginalia'


def watched(s, form, done, seconds, typed):
    """Eval FORM, which must return at once, then type a key per round and read
    DONE until it holds: the elapsed time and the round trips while waiting."""
    started = time.time()
    s.eval(form)
    returned = time.time() - started
    trips = []
    while time.time() - started < seconds:
        s.keys('-k', 'x')
        typed[0] += 1
        round_trip = time.time()
        answer = s.eval(done)
        trips.append(time.time() - round_trip)
        if answer != 'nil':
            return dict(returned=returned, elapsed=time.time() - started, max_round_trip=max(trips),
                        rounds=len(trips))
        time.sleep(.2)
    raise RuntimeError(f'{form} did not finish: {s.log()[-2000:]}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--tools', default=parity_live.TOOLS)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    report = {}
    with tempfile.TemporaryDirectory(prefix='packages-live-', dir=args.output) as base:
        s = Server(args, Path(base), 'pkg')
        try:
            path = s.file('scratch.txt', '')
            s.client(str(path), '-n')
            s.keyboard_ready()
            s.eval(f"(progn (require 'package) {ARCHIVES} (package-initialize) (setq romacs-test-buffer (current-buffer)) t)")
            # What a user does, in a clean HOME with no package lists yet.
            s.keys('-M', 'alt', '-k', 'x', '-m', 'alt')
            s.keys('package-install')
            s.keys('-k', 'Return')
            until(lambda: s.eval('(and (minibufferp (window-buffer (minibuffer-window))) (> (length package-archive-contents) 100) (string-prefix-p "Install package" (minibuffer-prompt)))') == 't',
                  60, 'the package prompt with the lists fetched')
            s.keys(PACKAGE)
            time.sleep(.5)
            s.keys('-k', 'Return')
            started = time.time()
            until(lambda: s.eval('(minibufferp (window-buffer))') == 'nil', 10, 'the prompt to close')
            report['prompt_closed'] = time.time() - started
            typed = [0]
            trips = []
            while s.eval(f"(package-installed-p '{PACKAGE})") == 'nil' and time.time() - started < 200:
                s.keys('-k', 'x')
                typed[0] += 1
                trip = time.time()
                s.eval('t')
                trips.append(time.time() - trip)
                time.sleep(.2)
            report['install'] = dict(elapsed=time.time() - started, max_round_trip=max(trips), rounds=len(trips))
            until(lambda: 'Magit installed \u2705' in s.eval('(with-current-buffer "*Messages*" (buffer-string))'),
                  10, 'the notification')
            report['notified'] = True
            # Typing goes on: the notification outlasts the next command.
            s.keys('-k', 'x')
            typed[0] += 1
            time.sleep(.25)
            report['toast'] = s.eval('(mapcar (lambda (toast) (with-current-buffer (car toast) (buffer-string))) romacs-layer--toasts)')
            subprocess.run([f'{args.tools}/grim', str(args.output / 'toast.png')], env=s.env, check=False, timeout=15)
            report['vc'] = watched(s, f'(progn (package-vc-install "{GIT_URL}") t)',
                                   f"(package-installed-p '{GIT_PACKAGE})", 120, typed)
            until(lambda: s.eval('(with-current-buffer romacs-test-buffer (buffer-size))') == str(typed[0]),
                  10, f'{typed[0]} typed keys in the buffer')
            report['typed'] = typed[0]
            report['require'] = s.eval(f"(progn (require '{PACKAGE}) (require '{GIT_PACKAGE}) (list (fboundp 'magit-status) (fboundp 'marginalia-mode)))")
            report['elc'] = s.eval('(length (directory-files-recursively package-user-dir "\\\\.elc\\\\\'"))')
            report['user_dir'] = s.eval('package-user-dir')
            report['selected'] = s.eval('package-selected-packages')
            report['ok'] = report['elc'] == '0' and report['require'] == '(t t)' and report['prompt_closed'] < 1 and 'Magit installed' in report['toast']
        finally:
            (args.output / 'server.log').write_text(s.log())
            s.close()
    (args.output / 'report.json').write_text(json.dumps(report, indent=1))
    print(json.dumps(report))
    print('PASS' if report.get('ok') else 'FAIL')
    return 0 if report.get('ok') else 1


if __name__ == '__main__':
    sys.exit(main())
