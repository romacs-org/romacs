#!/usr/bin/env python3
"""Raw bytes (GNU's eight-bit characters) in pinned GNU and in Romacs GUI
--loadup, side by side.

Each workflow starts a fresh pair of sessions (parity_live.py): pinned GNU
Emacs (pgtk) and `romacs-vm-gui --loadup', each in its own private headless
labwc with a private HOME/XDG/TMPDIR, both started in the same small work
directory holding a plain file, a file whose name is non-ASCII UTF-8, and a
file whose name is not UTF-8 (the byte 0xE9).

The journeys: `message' of a string with a raw byte (the echo area and
*Messages* show it as \\351), and `dired' of the work directory, then the
raw-named file visited with RET (listing text, buffer names, the visit).

usage: raw_bytes_parity_live.py --romacs BIN [--gnu EMACS] --output DIR
                                [--workflow NAME]... [--debug-on-error]

COMPARISON POLICY: parity_live.py's -- buffers, point, text, properties,
overlays, modes, mode and header lines, windows, minibuffer, echo area and
*Messages* exactly; paths, wall-clock and geometry normalised or reported
apart. Both private roots have one length, so buffer positions after a
path compare, and the whole-buffer text hash is taken with the root as
<root>.
"""
import os
import sys

from parity_live import (ROOT, Harness, K, M, MODE_IS, T, gnu_engine, key, lisp_string, main, mx,
                         private_tree, romacs_engine, typed)
import parity_live

RAW_NAME = os.fsdecode(b'\xe9x.txt')


def init_source(engine, root, debug):
    """The init both engines load: the same but for GNU's chrome and Romacs's
    defaults, which GNU takes as gnosis_parity_live.py's GNU does."""
    gnu_setup = ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
                 '(setq scroll-conservatively 101)\n'
                 f'(load {lisp_string(str(ROOT / "rlisp/romacs-completion-defaults.el"))} nil t)\n') if engine == 'gnu' else ''
    return f''';; -*- lexical-binding: t -*-
{gnu_setup}(setq inhibit-startup-screen t initial-scratch-message nil)
{'(setq debug-on-error t)' if debug else ''}
(setq parity-probe-variables nil)
(load {lisp_string(str(parity_live.PROBE))} nil t)
(defun raw-bytes-parity-snapshot ()
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
    (work / 'plain.txt').write_text('plain\n')
    (work / 'é.txt').write_text('accent\n')
    (work / RAW_NAME).write_text('raw name\n')


def launch(engine, base, args):
    # Roots of one length, so buffer positions after a path compare.
    root, home, env = private_tree(engine, base, args.tools, f'{engine}-'.ljust(len('romacs-'), '_'))
    work_tree(root)
    init = init_source(engine, root, args.debug_on_error)
    if engine == 'gnu':
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools)


# --- workflows ----------------------------------------------------------------

def evaluated(name, form, ready):
    """M-: FORM, typed, and the RET that evaluates it as the timed trigger."""
    return {'name': name, 'keys': [M(':'), T(form)],
            'keys_ready': f'(equal (minibuffer-contents-no-properties) {lisp_string(form)})',
            'trigger': [K('Return')], 'ready': ready}


MESSAGE_FORM = '(message "%s" (string #x3fffe9 ?x))'
VISIT_FORM = '(progn (goto-char (point-min)) (re-search-forward "x\\\\.txt$") (dired-move-to-filename) (dired-find-file))'

WORKFLOWS = {
    'message': [
        evaluated('message', MESSAGE_FORM, '(and (not (minibufferp)) (stringp (current-message)))'),
    ],
    'dired': [
        dict(name='open', **mx('dired'), ready='(minibufferp)'),
        key('directory', [K('Return')], MODE_IS('dired-mode')),
        evaluated('visit', VISIT_FORM, '(string-suffix-p "x.txt" (buffer-name))'),
    ],
}

HARNESS = Harness(WORKFLOWS, launch, '(raw-bytes-parity-snapshot)', 'raw-bytes-parity-')


def arguments(parser):
    parser.add_argument('--debug-on-error', action='store_true', help='set debug-on-error in both inits')


if __name__ == '__main__':
    sys.exit(main(HARNESS, 'raw bytes in GNU and Romacs, side by side', arguments))
