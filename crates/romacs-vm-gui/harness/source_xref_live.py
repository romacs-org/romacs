#!/usr/bin/env python3
"""M-. and C-h f on what Rust defines, with real keys in a --loadup session.

In an Emacs Lisp buffer, M-. on `read-string' (a defsubr), `car' (a legacy
primitive) and `fill-column' (a variable Rust forwards) visits each one's
Rust registration, in the source tree `source-directory' names, at its
line; M-, comes back.  On `when' it visits subr.el, as GNU's does.  C-h f
read-string says where the primitive is defined, and its button opens that
line.  Every step must hold; a signal is a failure.

usage: source_xref_live.py --binary BIN --output DIR
"""
import argparse
import json
import os
from pathlib import Path
import time

from parity_live import C, K, M, T, lisp_string, private_tree, romacs_engine
import parity_live

INIT = f''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(defvar source-xref-errors nil "What each command that signalled said.")
(setq command-error-function
      (let ((old command-error-function))
        (lambda (data context signal)
          (push (list this-command (error-message-string data)) source-xref-errors)
          (funcall old data context signal))))
(load {lisp_string(str(parity_live.PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''
# Where point is: the visited file relative to the source tree, the line,
# and the line's text.
WHERE = ('(list (and buffer-file-name (file-relative-name buffer-file-name source-directory))'
         ' (line-number-at-pos) (buffer-substring-no-properties (line-beginning-position) (line-end-position)))')
JOURNEY = '(progn (switch-to-buffer (get-buffer-create "*xref-journey*")) (delete-other-windows) (emacs-lisp-mode)' \
          ' (erase-buffer) (insert "(read-string)\\n(car)\\nfill-column\\n(when)\\n") t)'


def at(engine, word):
    # Each name where its namespace is unambiguous: a function's at a
    # form's head, a variable's alone (elisp-mode infers it from there).
    engine.ask(f'(with-current-buffer "*xref-journey*" (goto-char (point-min)) (search-forward {lisp_string(word)}) (backward-char 2) t)')


def wait(engine, form, timeout=15):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = engine.ask(form)
        if value not in (None, [], False):
            return value
        time.sleep(.2)
    return engine.ask(form)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', required=True)
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--tools', default=parity_live.TOOLS)
    a = ap.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    report = {'binary': a.binary, 'checks': [], 'failures': []}

    def check(name, ok, detail):
        report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
        if not ok:
            report['failures'].append(name)

    root, home, env = private_tree('romacs', Path(os.environ.get('TMPDIR', '/tmp')), a.tools)
    engine = romacs_engine(a.binary, root, home, env, INIT, None, 60, a.tools)
    try:
        tree = engine.ask('source-directory')
        check('source-directory names the source tree', isinstance(tree, str) and Path(tree, 'crates/romacs-vm/src/prims').is_dir(), tree)
        engine.ask(JOURNEY)
        for word, file, text in (('read-string', 'crates/romacs-vm/src/prims/minibuf.rs', 'name = "read-string"'),
                                 ('car', 'crates/romacs-vm/src/lib.rs', '("car", Primitive::'),
                                 ('fill-column', 'crates/romacs-vm/src/forwarded.rs', 'CVariable { name: "fill-column"'),
                                 ('when', 'gnu-lisp/subr.el', '(defmacro when ')):
            at(engine, word)
            engine.keys(M('.'))
            where = wait(engine, f'(and (not (equal (buffer-name) "*xref-journey*")) {WHERE})')
            seen = engine.ask('(list (buffer-name) (point) (current-message) (with-current-buffer "*xref-journey*" (point)))')
            check(f'M-. on {word} visits {file}', bool(where) and where[0] == file and text in where[2], [where, seen])
            engine.keys(M(','))
            back = wait(engine, '(equal (buffer-name) "*xref-journey*")')
            check(f'M-, from {word} comes back', back in (True, 't'), back)
        engine.keys(C('h'), T('f'))
        wait(engine, '(minibufferp)')
        engine.keys(T('read-string'), K('Return'))
        shown = wait(engine, '(and (get-buffer "*Help*") (with-current-buffer "*Help*" (goto-char (point-min))'
                             ' (and (search-forward "minibuf.rs" nil t) (let ((b (button-at (1- (point)))))'
                             ' (list (buffer-substring-no-properties (line-beginning-position) (line-end-position)) (and b (symbol-name (button-type b))))))))')
        check('C-h f read-string names its Rust file as a button',
              bool(shown) and 'crates/romacs-vm/src/prims/minibuf.rs' in shown[0] and shown[1] == 'help-function-def', shown)
        engine.ask('(progn (select-window (get-buffer-window "*Help*")) (goto-char (point-min)) (search-forward "minibuf.rs") (backward-char 2) t)')
        engine.keys(K('Return'))
        opened = wait(engine, f'(and (equal (file-name-nondirectory (or buffer-file-name "")) "minibuf.rs") {WHERE})')
        check('its button opens the registration', bool(opened) and 'name = "read-string"' in opened[2], opened)
        errors = engine.ask('source-xref-errors')
        check('no command signalled', errors in (None, []), errors)
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
        report['failures'].append('error')
    finally:
        engine.stop()
    report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    (a.output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({'failures': report['failures'], 'error': report.get('error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
