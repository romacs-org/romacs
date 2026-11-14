#!/usr/bin/env python3
"""Eshell in pinned GNU and in Romacs GUI --loadup, side by side.

Each workflow starts a fresh pair of sessions (parity_live.py): pinned GNU
Emacs (pgtk) and `romacs-vm-gui --loadup', each in its own private headless
labwc with a private HOME/XDG/TMPDIR, both started in the same small work
directory (two files, a directory and a 300-line file).  Eshell keeps its
history and state under `eshell-directory-name', set to the private tree in
both, so neither reads or writes the user's.

The journey: `M-x eshell' from *scratch*, Eshell's own `ls', `echo', `cd'
and `pwd', `cat' of a file several screens long, external `printf', an
external `sleep 5' interrupted with C-c C-c, history with M-p, killing that
input with C-c C-u, and `exit'.

usage: eshell_parity_live.py --romacs BIN [--gnu EMACS] --output DIR
                             [--workflow NAME]... [--perf] [--debug-on-error]
--debug-on-error sets `debug-on-error' in both inits, so a signal shows its
*Backtrace* window in the snapshot that follows (the windows' text).

COMPARISON POLICY: as gnosis_parity_live.py's -- buffers, point, text,
properties, overlays, modes, mode and header lines, windows, minibuffer,
echo area and *Messages* exactly; paths, wall-clock and geometry
normalised or reported apart -- with the package state the probe records
being `PROBE_VARIABLES' below: Eshell's input and output markers, its
command and history state and `default-directory'.  Both private roots
have one length, so buffer positions after a path compare, and the
whole-buffer text hash is taken with the root as <root>.
"""
import sys

from parity_live import (ROOT, C, Harness, K, M, MODE_IS, gnu_engine, key, lisp_string, main, mx,
                         private_tree, romacs_engine, typed)
import parity_live

PROBE_VARIABLES = [
    'default-directory', 'eshell-last-input-start', 'eshell-last-input-end', 'eshell-last-output-start',
    'eshell-last-output-block-begin', 'eshell-last-output-end', 'eshell-foreground-command',
    'eshell-last-command-status', 'eshell-last-command-name', 'eshell-history-index', 'eshell-process-list',
]


def init_source(engine, root, debug):
    """The init both engines load: the same but for GNU's chrome and Romacs's
    defaults, which GNU takes as gnosis_parity_live.py's GNU does."""
    gnu_setup = ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
                 '(setq scroll-conservatively 101)\n'
                 f'(load {lisp_string(str(ROOT / "lisp/romacs/romacs-completion-defaults.el"))} nil t)\n') if engine == 'gnu' else ''
    variables = ' '.join(PROBE_VARIABLES)
    return f''';; -*- lexical-binding: t -*-
{gnu_setup}{parity_live.GNU_COMPLETION}(setq inhibit-startup-screen t initial-scratch-message nil)
(setq eshell-directory-name {lisp_string(str(root / 'eshell') + '/')})
{'(setq debug-on-error t)' if debug else ''}
(setq parity-probe-variables '({variables}))
(random "eshell-parity")
(load {lisp_string(str(parity_live.PROBE))} nil t)
(defun eshell-parity-snapshot ()
  "The probe's snapshot, its text hash taken with the private root as <root>."
  (let ((snapshot (parity-probe-snapshot)))
    (with-current-buffer (plist-get snapshot :buffer)
      (plist-put snapshot :text-hash
                 (secure-hash 'sha1 (string-replace {lisp_string(str(root))} "<root>"
                                                    (buffer-substring-no-properties (point-min) (point-max))))))))
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def work_tree(root):
    """The directory both engines start in."""
    work = root / 'work'
    (work / 'alpha.txt').write_text('alpha\n')
    (work / 'beta.el').write_text(';; beta\n')
    (work / 'gamma').mkdir()
    (work / 'lines.txt').write_text(''.join(f'line {i}\n' for i in range(1, 301)))


def launch(engine, base, args):
    # Roots of one length, so buffer positions after a path the prompt or
    # `pwd' shows compare between the engines.
    root, home, env = private_tree(engine, base, args.tools, f'{engine}-'.ljust(len('romacs-'), '_'))
    work_tree(root)
    init = init_source(engine, root, args.debug_on_error)
    if engine == 'gnu':
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools)


# --- workflows ----------------------------------------------------------------

ESHELL = lisp_string('*eshell*')
# Eshell is waiting at a fresh prompt: no foreground command, point at the
# end, just after a prompt.
IDLE = (f'(and {MODE_IS("eshell-mode")} (not eshell-foreground-command) (= (point) (point-max))'
        ' (> (point) 1) (eq (get-text-property (1- (point)) (quote field)) (quote prompt)))')
RUNNING = f'(and {MODE_IS("eshell-mode")} eshell-foreground-command t)'
INPUT_IS = ('(and {} (equal (buffer-substring-no-properties eshell-last-output-end (point-max)) {}))').format


def command(name, text):
    """Type TEXT at the prompt, then time the RET that runs it to the next prompt."""
    return typed(name, text, [K('Return')], IDLE)


WORKFLOWS = {
    'journey': [
        dict(name='open', **mx('eshell'), ready=IDLE),
        command('ls', 'ls'),
        command('echo', 'echo hi'),
        command('cd-up', 'cd ..'),
        command('pwd', 'pwd'),
        command('cat-screens', 'cat work/lines.txt'),
        command('printf', "printf 'a\\nb\\n'"),
        typed('sleep', 'sleep 5', [K('Return')], RUNNING),
        key('interrupt', [C('c'), C('c')], IDLE),
        key('history', [M('p')], INPUT_IS(MODE_IS('eshell-mode'), lisp_string('sleep 5'))),
        key('kill-input', [C('c'), C('u')], IDLE),
        typed('exit', 'exit', [K('Return')], f'(not (get-buffer {ESHELL}))'),
    ],
    # External output that is not UTF-8: the raw byte lands in the buffer
    # as GNU's eight-bit character (shown as \351).
    'raw': [
        dict(name='open', **mx('eshell'), ready=IDLE),
        command('printf-raw', "printf 'a\\xe9b\\n'"),
        typed('exit', 'exit', [K('Return')], f'(not (get-buffer {ESHELL}))'),
    ],
}

HARNESS = Harness(WORKFLOWS, launch, '(eshell-parity-snapshot)', 'eshell-parity-')


def arguments(parser):
    parser.add_argument('--debug-on-error', action='store_true', help='set debug-on-error in both inits')


if __name__ == '__main__':
    sys.exit(main(HARNESS, __doc__, arguments))
