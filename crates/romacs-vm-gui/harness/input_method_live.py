#!/usr/bin/env python3
"""Input methods in pinned GNU and Romacs, by real keys, side by side.

Each workflow starts a fresh pair of sessions (parity_live.py): pinned GNU
Emacs and `romacs-vm-gui --loadup', each in its own private headless labwc
with a private HOME/XDG/TMPDIR, both on the same init.el.  The keys go
through each engine's own command loop, typed by wtype, into a text-mode
buffer whose `default-input-method' is greek-polytonic.

  greek     C-\\ on, plain letters, dead-key sequences, a final sigma, C-\\ off.
  pending   a sequence left open: its overlay, then C-g, DEL and a key the
            method does not bind, which ends it and runs as a command.
  methods   C-x RET C-\\ to bulgarian-phonetic, TeX and chinese-py by name,
            their text, and chinese-py's candidate motion.
  help      `describe-input-method' and the mode line's indicator.
  reads     `read-string' and `read-char' with and without
            INHERIT-INPUT-METHOD, C-q, `read-passwd', C-u, a read-only buffer.
  isearch   C-s with the method on, `isearch-toggle-input-method', C-^.
  macro     F3 keys F4 F4: the macro holds the raw keys, and replays them
            through the method.

COMPARISON POLICY: after each step, exactly: the echo area, *Messages* since
the workflow began, without its load messages (GNU names a native-compiled
file), the bells rung, whether the buffer is modified, *Help* without the
rules of its keyboard table, whose length `current-column' sets
(VM2-CURRENT-COLUMN), and the Quail completions, the selected buffer, the recursion depth,
the minibuffer's prompt and contents, the buffer's text and point, the method
and its title, `input-method-function', the mode line's indicator, the
pending sequence, its overlay's span and face, `recent-keys' (BackSpace and
Tab as DEL and TAB: the GUI delivers their text) and the last macro.
Not compared: the echo of keys read inside a command (VM2-KBD1), the message a macro's last command left, which GNU has wiped by
the macro's end with or without a method, and the name of the function in the
logged error of a self-insert into a read-only buffer.

usage: input_method_live.py --romacs BIN [--gnu EMACS] --output DIR [--workflow NAME]...
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys
import time

from parity_live import C, K, M, T, differences, gnu_engine, lisp_string, plist, private_tree, romacs_engine, wait
import parity_live

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil default-input-method "greek-polytonic"
      suggest-key-bindings nil)
(defvar im-bells 0)
(defvar im-got nil)
(setq ring-bell-function (lambda () (setq im-bells (1+ im-bells))))
(global-set-key [f5] (lambda () (interactive) (setq im-got (read-string "Inherit: " nil nil nil t))))
(global-set-key [f6] (lambda () (interactive) (setq im-got (read-string "Plain: "))))
(global-set-key [f7] (lambda () (interactive) (setq im-got (list (read-char nil t) (read-char)))))
(global-set-key [f8] (lambda () (interactive) (setq im-got (read-passwd "Password: "))))
(global-set-key [f9] (lambda () (interactive) (setq im-got (read-key-sequence "Key: "))))
(load {probe} nil t)
(switch-to-buffer (get-buffer-create "im"))
(text-mode)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''

OBSERVE = '''(let* ((b (window-buffer (minibuffer-selected-window)))
       (cur (window-buffer (selected-window)))
       (keys (recent-keys))
       (tail (nthcdr (max 0 (- (length keys) 12)) (append keys nil))))
  (list
   :echo (let ((m (current-message))) (and m (substring-no-properties m)))
   :messages (parity-probe--messages)
   :bells im-bells
   :selected (buffer-name cur)
   :depth (recursion-depth)
   :prompt (minibuffer-prompt)
   :contents (and (minibufferp cur) (with-current-buffer cur (minibuffer-contents-no-properties)))
   :text (with-current-buffer b (buffer-substring-no-properties (point-min) (point-max)))
   :point (with-current-buffer b (point))
   :method (with-current-buffer cur (list current-input-method current-input-method-title input-method-function))
   :indicator (with-current-buffer b (substring-no-properties (format-mode-line mode-line-mule-info)))
   :pending (and (boundp 'quail-current-key) (list quail-current-key quail-current-str quail-translating quail-converting))
   :overlay (with-current-buffer cur
              (and (boundp 'quail-overlay) (overlayp quail-overlay) (overlay-buffer quail-overlay)
                   (list (overlay-start quail-overlay) (overlay-end quail-overlay) (overlay-get quail-overlay 'face))))
   :got im-got
   :keys (mapcar (lambda (k) (pcase k ('backspace 127) ('tab 9) (_ k))) tail)
   :modified (with-current-buffer b (buffer-modified-p))
   :help (let ((h (get-buffer "*Help*")))
           (and h (with-current-buffer h
                    (seq-take (seq-remove (lambda (line) (string-match-p "^[ \t]*[+]-+[+]$" line))
                                          (split-string (buffer-substring-no-properties (point-min) (point-max)) "\n"))
                              24))))
   :completions (let ((h (get-buffer "*Quail Completions*"))) (and h (get-buffer-window h) (with-current-buffer h (buffer-substring-no-properties (point-min) (min (point-max) 600)))))
   :macro (list defining-kbd-macro last-kbd-macro)
   :search (and (boundp 'isearch-mode) isearch-mode (list isearch-string isearch-message))))'''

TEXT_IS = '(equal (with-current-buffer "im" (buffer-string)) {})'.format
TEXT = lambda text: TEXT_IS(lisp_string(text))
METHOD_IS = '(equal current-input-method {})'.format
METHOD = lambda name: METHOD_IS(lisp_string(name))
KEY_PENDING = '(and (boundp (quote quail-current-key)) (equal quail-current-key {}))'.format
PENDING = lambda keys: KEY_PENDING(lisp_string(keys))
PROMPT_STARTS = lambda text: f'(string-prefix-p {lisp_string(text)} (or (minibuffer-prompt) ""))'
CONTENTS = lambda text: f'(equal (minibuffer-contents-no-properties) {lisp_string(text)})'
GOT = lambda text: f'(equal im-got {lisp_string(text)})'
NOT_READING = '(not (minibufferp))'
ON = ('toggle-on', [C('\\')], METHOD('greek-polytonic'))
OFF = ('toggle-off', [C('\\')], '(null current-input-method)')
SET_METHOD = lambda name: [C('x'), K('Return'), C('\\'), T(name), K('Return')]


WORKFLOWS = {
    'greek': [
        ON,
        ('letters', [T('abg')], TEXT('αβγ')),
        ('word', [T(' logos')], TEXT('αβγ λογοσ')),
        ('final-sigma', [T(' logow')], TEXT('αβγ λογοσ λογος')),
        ('newline', [K('Return')], '(string-suffix-p "\n" (with-current-buffer "im" (buffer-string)))'),
        ('acute', [T("'a")], 't'),
        ('grave', [T('`e')], 't'),
        ('circumflex', [T('~h')], 't'),
        ('breathing', [T(':a')], 't'),
        ('undo-mark', [C('e')], 't'),
        ('undo-keys', [T('ab')], 't'),
        ('undo', [C('_')], 't'),
        OFF,
        ('ascii', [T('abc')], '(string-suffix-p "abc" (with-current-buffer "im" (buffer-string)))'),
    ],
    'pending': [
        ON,
        ('open', [T("'")], PENDING("'")),
        ('abort', [C('g')], 't'),
        ('open-again', [T("'")], PENDING("'")),
        ('delete', [K('BackSpace')], 't'),
        ('text', [T('ab')], TEXT('῀αβ')),
        ('open-third', [T("'")], PENDING("'")),
        ('other-command', [C('a')], 't'),
        ('after', [T('g')], 't'),
        ('prefix-key', [C('x'), T('h')], 't'),
    ],
    'methods': [
        ('bulgarian-on', SET_METHOD('bulgarian-phonetic'), METHOD('bulgarian-phonetic')),
        ('bulgarian-text', [T('abvgd shch')], TEXT('абжгд схцх')),
        ('to-tex', [C('u'), C('\\'), T('TeX'), K('Return')], METHOD('TeX')),
        ('tex-text', [T(' \\alpha x^2 \\to ')], '(string-suffix-p "→ " (with-current-buffer "im" (buffer-string)))'),
        ('to-chinese', SET_METHOD('chinese-py'), METHOD('chinese-py')),
        ('pinyin-open', [T('ni')], PENDING('ni')),
        ('pinyin-next', [C('f')], 't'),
        ('pinyin-previous', [C('b')], 't'),
        ('pinyin-block', [C('n')], 't'),
        ('pinyin-commit', [T('1')], 't'),
        ('pinyin-complete-open', [T('zh')], PENDING('zh')),
        ('pinyin-complete', [K('Tab')], '(get-buffer-window "*Quail Completions*")'),
        ('pinyin-end', [C('g')], 't'),
        ('to-hangul', SET_METHOD('korean-hangul'), METHOD('korean-hangul')),
        ('hangul-text', [T('dkssud ')], 't'),
        ('hangul-delete', [T('gks'), K('BackSpace'), T(' ')], 't'),
    ],
    'help': [
        ON,
        ('describe', [C('h'), T('I'), T('greek-polytonic'), K('Return')], '(get-buffer "*Help*")'),
    ],
    'reads': [
        ON,
        ('inherit-prompt', [K('F5')], PROMPT_STARTS('Inherit')),
        ('inherit-text', [T('ab')], CONTENTS('αβ')),
        ('inherit-exit', [K('Return')], GOT('αβ')),
        ('pending-prompt', [K('F5')], PROMPT_STARTS('Inherit')),
        ('pending-open', [T("'")], PENDING("'")),
        ('pending-quit', [C('g')], '(= (minibuffer-depth) 0)'),
        ('plain-prompt', [K('F6')], PROMPT_STARTS('Plain')),
        ('plain-text', [T('ab')], CONTENTS('ab')),
        ('plain-exit', [K('Return')], GOT('ab')),
        ('read-char', [K('F7')], 't'),
        ('read-char-keys', [T('ab')], f'(equal im-got (list {ord("α")} ?b))', 'echo'),
        ('quoted', [C('q'), T('a')], TEXT('a')),
        ('password', [K('F8')], PROMPT_STARTS('Password')),
        ('password-keys', [T('ab'), K('Return')], GOT('ab')),
        ('key-prompt', [K('F9')], 't'),
        ('key-read', [T('a')], 't', 'echo'),
        ('prefix-arg', [C('u'), T('3a')], 't'),
        ('read-only', [M('x'), T('read-only-mode'), K('Return')], 'buffer-read-only'),
        ('refused', [T('a')], 't', 'messages'),
    ],
    'isearch': [
        ('seed', [C('\\'), T('abg abg'), M('<')], '(= (point) 1)'),
        ('search', [C('s')], 'isearch-mode'),
        ('search-text', [T('ab')], '(equal isearch-string "αβ")'),
        ('search-exit', [K('Return')], '(not isearch-mode)'),
        ('search-off', [C('s'), C('\\')], 't'),
        ('search-ascii', [T('g')], 't'),
        ('search-on', [C('\\')], 't'),
        ('search-more', [T('g')], 't'),
        ('search-unfail', [C('g')], 't'),
        ('search-quit', [C('g')], '(not isearch-mode)'),
    ],
    'macro': [
        ON,
        ('start', [K('F3')], 'defining-kbd-macro'),
        ('keys', [T("ab'a")], 't'),
        ('end', [K('F4')], '(not defining-kbd-macro)'),
        ('call', [K('F4')], 't', 'echo'),
    ],
}


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools)
    init = INIT.format(probe=lisp_string(str(parity_live.PROBE)))
    if engine == 'gnu':
        init += '(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools)


def ready(engine, predicate, what):
    wait(lambda: plist(engine.ask(f'(parity-probe-ready (lambda () {predicate}))')).get('ready'),
         engine.timeout, f'{engine.name} {what}', engine.process, .05)


def observed(engine):
    """OBSERVE's answer, as a dict, with GNU's load messages left out."""
    fields = plist(engine.ask(OBSERVE))
    fields['messages'] = [m for m in fields['messages'] or [] if not m.startswith('Loading ')]
    if (fields['echo'] or '').startswith('Loading '):
        fields['echo'] = None
    return fields


def run_steps(engines, steps, entry):
    for engine in engines:
        engine.ask('(progn (parity-probe-mark-messages) t)')
    for name, keys, predicate, *uncompared in steps:
        record = {'step': name, 'observed': {}}
        entry['steps'].append(record)
        for engine in engines:
            engine.keys(*keys)
            ready(engine, predicate, name)
            time.sleep(.3)
            record['observed'][engine.name] = observed(engine)
        gnu, romacs = (sorted((k, v) for k, v in record['observed'][name].items() if k not in uncompared)
                       for name in ('gnu', 'romacs'))
        record['differences'] = differences(gnu, romacs)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=parity_live.os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=parity_live.os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append', choices=list(WORKFLOWS))
    parser.add_argument('--timeout', type=float, default=60)
    parser.add_argument('--keep', action='store_true')
    args = parser.parse_args()
    parity_live.GNU = args.gnu
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'romacs_sha256': hashlib.sha256(Path(args.romacs).read_bytes()).hexdigest(),
              'gnu': args.gnu, 'workflows': []}
    for name in args.workflow or list(WORKFLOWS):
        entry = {'workflow': name, 'steps': [], 'status': 'FAIL'}
        report['workflows'].append(entry)
        engines = []
        try:
            for engine in ('gnu', 'romacs'):
                engines.append(launch(engine, Path(args.work), args))
            run_steps(engines, WORKFLOWS[name], entry)
            entry['status'] = 'PASS' if not any(s.get('differences', True) for s in entry['steps']) else 'DIFFER'
        except Exception as error:  # noqa: BLE001
            entry['error'] = repr(error)
        finally:
            for engine in engines:
                for log in ('engine.log', 'compositor.log'):
                    if (engine.root / log).exists():
                        shutil.copy(engine.root / log, output / f'{name}-{engine.name}-{log}')
                engine.stop()
                if not args.keep:
                    shutil.rmtree(engine.root, ignore_errors=True)
        print(name, entry['status'], entry.get('error', ''),
              json.dumps([{'step': s['step'], 'differences': s.get('differences')} for s in entry['steps'] if s.get('differences', True)],
                         ensure_ascii=False)[:3000], flush=True)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    return 0 if all(w['status'] == 'PASS' for w in report['workflows']) else 1


if __name__ == '__main__':
    sys.exit(main())
