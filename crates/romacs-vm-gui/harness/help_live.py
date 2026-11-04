#!/usr/bin/env python3
"""Help commands in pinned GNU and in Romacs GUI --loadup, side by side.

Each workflow starts a fresh pair of sessions (parity_live.py): pinned GNU
Emacs (pgtk) and `romacs-vm-gui --loadup', each in its own private headless
labwc with a private HOME/XDG/TMPDIR, both on the same init.el.  The keys
go through each engine's own command loop, typed by wtype, in *scratch*:

  bindings  C-h b: describe-bindings, over describe-buffer-bindings and
            help--describe-vector (rlisp/romacs-help.el).
  mode      C-h m: describe-mode, its minor modes and the major mode's map.
  key       C-h k C-f: describe-key, then C-h k C-x C-f, a prefix key.
  function  C-h f car RET and C-h f describe-vector RET: describe-function.
  syntax    M-x describe-syntax RET: describe-vector over the syntax table
            with internal-describe-syntax-value.  Not run by default: the
            table shows U+2FFF, which no font of the gate's covers, and an
            uncovered character still ends a Romacs session (layout2's
            `unresolved face text'); name it to run it.

COMPARISON POLICY: parity_live.py's -- buffers, point, text, properties,
overlays, modes, windows, minibuffer, echo area and *Messages* exactly;
paths, wall-clock and geometry normalised or reported apart.  The *Help*
text is compared too and its differences reported; a workflow FAILs on any
*Messages* line Romacs logs that GNU does not (an error in a help command
is one) or on a command error GNU does not signal (both inits record
them through `command-error-function'), and PASSes only when nothing
differs.  Both run without Romacs's defaults, completion and mode line
(parity_live.GNU_COMPLETION).

usage: help_live.py --romacs BIN [--gnu EMACS] --output DIR [--workflow NAME]...
"""
import sys

from parity_live import (C, K, M, T, gnu_engine, key, lisp_string, main, mx, plist, private_tree, romacs_engine)
import parity_live

INIT = ''';; -*- lexical-binding: t -*-
{gnu}{completion}(setq inhibit-startup-screen t initial-scratch-message nil)
(defvar help-live-errors nil)
(let ((default command-error-function))
  (setq command-error-function
        (lambda (data context caller)
          (push (error-message-string data) help-live-errors)
          (funcall default data context caller))))
(setq parity-probe-variables nil)
(load {probe} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools, f'{engine}-'.ljust(len('romacs-'), '_'))
    gnu = '(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n' if engine == 'gnu' else ''
    init = INIT.format(gnu=gnu, completion=parity_live.GNU_COMPLETION, probe=lisp_string(str(parity_live.PROBE)))
    if engine == 'gnu':
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools)


HELP_SHOWS = ('(let ((b (get-buffer "*Help*"))) (and b (get-buffer-window b)'
              ' (with-current-buffer b (save-excursion (goto-char (point-min)) (search-forward {} nil t)))))').format
QUIT_HELP = key('quit-help', [C('x'), T('1')], '(not (get-buffer-window "*Help*"))')


def help_key(name, keys, shows):
    return key(name, keys, HELP_SHOWS(lisp_string(shows)))


def described(name, keys, prompt_keys, shows):
    """KEYS's prompt, PROMPT_KEYS typed into it untimed, RET as the trigger."""
    return {'name': name, 'keys': keys + prompt_keys, 'keys_ready': '(minibufferp)',
            'trigger': [K('Return')], 'ready': HELP_SHOWS(lisp_string(shows))}


WORKFLOWS = {
    'bindings': [help_key('describe-bindings', [C('h'), T('b')], 'Global Bindings'), QUIT_HELP],
    'mode': [help_key('describe-mode', [C('h'), T('m')], 'Lisp Interaction'), QUIT_HELP],
    'key': [help_key('describe-key', [C('h'), T('k'), C('f')], 'forward-char'), QUIT_HELP,
            help_key('describe-prefix-key', [C('h'), T('k'), C('x'), C('f')], 'find-file'), QUIT_HELP],
    'function': [described('describe-car', [C('h'), T('f')], [T('car')], 'car'), QUIT_HELP,
                 described('describe-vector', [C('h'), T('f')], [T('describe-vector')], 'describe-vector'), QUIT_HELP],
    'syntax': [dict(mx('describe-syntax'), name='describe-syntax', ready=HELP_SHOWS(lisp_string('which means'))), QUIT_HELP],
}

def check(engines):
    """The command errors Romacs signalled that GNU did not, and the lines
    naming an error that Romacs's *Messages* gained and GNU's did not."""
    messages = {engine.name: engine.ask('(parity-probe--messages)') or [] for engine in engines}
    errors = {engine.name: engine.ask('(reverse help-live-errors)') or [] for engine in engines}
    extra = [e for e in errors['romacs'] if e not in errors['gnu']]
    extra += [line for line in messages['romacs'] if line not in messages['gnu'] and 'rror' in line]
    return {'messages': messages, 'command_errors': errors, 'errors': extra}


if __name__ == '__main__':
    sys.exit(main(parity_live.Harness(WORKFLOWS, launch, check=check, writes=('syntax',)), __doc__))
