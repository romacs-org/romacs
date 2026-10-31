#!/usr/bin/env python3
"""Command errors, the debugger and C-x C-c in pinned GNU and Romacs, side by side.

Each workflow starts a fresh pair of sessions (parity_live.py): pinned GNU
Emacs (pgtk) and `romacs-vm-gui --loadup', each in its own private headless
labwc with a private HOME/XDG/TMPDIR, both on the same init.el.  The keys
go through each engine's own command loop, typed by wtype.

  errors    commands that signal: `user-error', `error', a primitive's
            wrong-type-argument, C-f at the end of the buffer and C-g.
  macros    C-x ( ab C-x ) C-x e, then F3 x F4 F4: kmacro.el's keys over
            read_char's recording and the command loop's replay.
  kill-buffer
            C-x k RET on an unmodified file buffer, then on a modified one,
            whose kill GNU's `kill-buffer' asks to confirm.
  debugger  M-: (car 1), which `eval-expression' runs with the debugger on,
            then q; a command binding `debug-on-error' around (car 1), then
            c, which continues the error to the command loop.
  autosave  typing more than `auto-save-interval' characters into a visited
            buffer, and an idle wait of `auto-save-timeout', each make
            #FILE# (read_char's two auto-save triggers).
  sigterm   SIGTERM over a modified file buffer: `kill-emacs-hook', the
            auto-save and the exit status 15.
  quit      C-x C-c with a modified file buffer: the save prompt, y, then
            `kill-emacs-query-functions' and `kill-emacs-hook' and the
            exit status; and M-: (kill-emacs 3) for a status of 3.

COMPARISON POLICY: after each step, exactly: the echo area, the lines
*Messages* gained since the workflow began, the bells rung (a counting
`ring-bell-function'), the buffer list, the selected buffer, the buffers
the windows show, the recursion depth, the minibuffer prompt, the shown
buffer's text, the macro state, and *Backtrace*'s mode and first
two lines.  The rest of *Backtrace* (frames whose shapes the engines'
compilers decide, VM2-ERT-BATCH1) is recorded, not compared; so, in
kill-buffer, are the buffer list's order and the buffer that replaces a
killed one, which record-buffer and other-buffer decide (`recency_free').  Paths
become <root>.  For quit: the saved file's bytes, what the query function
and the hook recorded, and the process's exit status.

usage: command_errors_live.py --romacs BIN [--gnu EMACS] --output DIR [--workflow NAME]...
"""
import json
from pathlib import Path
import sys
import time

from parity_live import (C, K, M, T, gnu_engine, lisp_string, plist, private_tree, romacs_engine, wait)
import parity_live

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(defvar ce-bells 0)
(setq ring-bell-function (lambda () (setq ce-bells (1+ ce-bells))))
(defun ce-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil
                (expand-file-name "record.eld" parity-probe-dir) 'append 'silent))
(defun ce-user-error () (interactive) (user-error "Nothing to do here"))
(defun ce-error () (interactive) (error "Boom %d" 7))
(defun ce-car () (interactive) (car 1))
(defun ce-car-1 () (car 1))
(defun ce-debug-car () (interactive) (let ((debug-on-error t)) (ce-car-1)))
(global-set-key [f5] #'ce-user-error)
(global-set-key [f6] #'ce-error)
(global-set-key [f7] #'ce-car)
(global-set-key [f8] #'ce-debug-car)
(add-hook 'kill-emacs-query-functions (lambda () (ce-record '(query)) t))
(add-hook 'kill-emacs-hook (lambda () (ce-record '(hook))))
(load {probe} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''

OBSERVE = '''(list
 :echo (let ((m (current-message))) (and m (substring-no-properties m)))
 :messages (parity-probe--messages)
 :bells ce-bells
 :buffers (seq-remove (lambda (n) (string-prefix-p " " n)) (mapcar #'buffer-name (buffer-list)))
 :selected (buffer-name (window-buffer (selected-window)))
 :windows (mapcar (lambda (w) (buffer-name (window-buffer w))) (window-list nil 'nomini))
 :depth (recursion-depth)
 :prompt (minibuffer-prompt)
 :text (let ((b (window-buffer (minibuffer-selected-window))))
         (and (not (equal (buffer-name b) "*Backtrace*"))
              (with-current-buffer b (buffer-substring-no-properties (point-min) (point-max)))))
 :macro (list defining-kbd-macro last-kbd-macro)
 :backtrace (let ((b (get-buffer "*Backtrace*")))
              (and b (with-current-buffer b
                       (list major-mode (buffer-substring-no-properties
                                         (point-min) (save-excursion (goto-char (point-min)) (line-end-position 2))))))))'''
BACKTRACE_TEXT = '(let ((b (get-buffer "*Backtrace*"))) (and b (with-current-buffer b (buffer-substring-no-properties (point-min) (min (point-max) 3000)))))'

ECHO_IS = '(equal (current-message) {})'.format
DEBUGGING = '(and (eq major-mode (quote debugger-mode)) (> (recursion-depth) 0))'
EVALUATING = '(equal (minibuffer-contents-no-properties) {})'.format


TEXT_IS = '(equal (buffer-substring-no-properties (point-min) (point-max)) {})'.format
PROMPT_STARTS = '(string-prefix-p {} (or (minibuffer-prompt) ""))'.format


def step(name, keys, ready='t', **extra):
    return dict(name=name, keys=keys, ready=ready, **extra)


WORKFLOWS = {
    'errors': [
        step('user-error', [K('F5')], ECHO_IS(lisp_string('Nothing to do here'))),
        step('error', [K('F6')], ECHO_IS(lisp_string('Boom 7'))),
        step('wrong-type-argument', [K('F7')], ECHO_IS(lisp_string('Wrong type argument: listp, 1'))),
        step('end-of-buffer', [M('>'), C('f')], ECHO_IS(lisp_string('End of buffer'))),
        step('quit', [C('g')], ECHO_IS(lisp_string('Quit'))),
    ],
    'macros': [
        step('start', [C('x'), T('(')], 'defining-kbd-macro'),
        step('type', [T('ab')], TEXT_IS(lisp_string('ab'))),
        step('end', [C('x'), T(')')], '(and (not defining-kbd-macro) last-kbd-macro)'),
        step('call', [C('x'), T('e')], TEXT_IS(lisp_string('abab'))),
        step('f3', [K('F3')], 'defining-kbd-macro'),
        step('f3-type', [T('x')], TEXT_IS(lisp_string('ababx'))),
        step('f4-end', [K('F4')], '(not defining-kbd-macro)'),
        step('f4-call', [K('F4')], TEXT_IS(lisp_string('ababxx'))),
    ],
    'kill-buffer': [
        step('prompt', [C('x'), T('k')], PROMPT_STARTS(lisp_string('Kill buffer')), recency=True),
        step('kill', [K('Return')], '(not (get-buffer "notes.txt"))', recency=True),
        step('modified-prompt', [C('x'), T('k'), T('draft.txt'), K('Return')], PROMPT_STARTS(lisp_string('Buffer')),
             recency=True),
        step('modified-yes', [T('yes'), K('Return')], '(not (get-buffer "draft.txt"))', recency=True),
    ],
    'debugger': [
        step('eval-expression', [M(':'), T('(car 1)')], EVALUATING(lisp_string('(car 1)')), then=[K('Return')],
             then_ready=DEBUGGING),
        step('debugger-quit', [T('q')], '(= (recursion-depth) 0)'),
        step('debug-on-error-command', [K('F8')], DEBUGGING),
        step('debugger-continue', [T('c')], '(= (recursion-depth) 0)'),
    ],
}


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools)
    init = INIT.format(probe=lisp_string(str(parity_live.PROBE)))
    if engine == 'gnu':
        init += '(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools)


def normalise(value, engine):
    if isinstance(value, list):
        return [normalise(v, engine) for v in value]
    if isinstance(value, str):
        return value.replace(str(engine.root), '<root>')
    return value


def ready(engine, predicate, what):
    wait(lambda: plist(engine.ask(f'(parity-probe-ready (lambda () {predicate}))')).get('ready'),
         engine.timeout, f'{engine.name} {what}', engine.process, .05)


def kill_buffer_setup(engines):
    """Two file buffers: notes.txt shown, unmodified, and draft.txt, modified,
    which shows once notes.txt is gone."""
    for engine in engines:
        for name, text in (('draft.txt', 'draft\n'), ('notes.txt', 'notes\n')):
            (engine.root / 'work' / name).write_text(text)
            engine.ask(f'(progn (find-file {lisp_string(str(engine.root / "work" / name))}) t)')
        engine.ask('(progn (with-current-buffer "draft.txt" (insert "edit ")) (switch-to-buffer "draft.txt")'
                   ' (switch-to-buffer "notes.txt") t)')


SETUPS = {'kill-buffer': kill_buffer_setup}


def recency_free(observed):
    """OBSERVED without what the buffer list's recency decides: the buffers
    as a set, and not which buffer a window shows instead of a killed one
    or what it holds (record-buffer and other-buffer belong to the buffer
    list's owners)."""
    fields = parity_live.plist(observed)
    fields['buffers'] = sorted(fields['buffers'])
    for name in ('windows', 'text') if fields.get('prompt') else ('selected', 'windows', 'text'):
        fields.pop(name, None)
    return sorted(fields.items())


def run_steps(engines, steps, entry, name=None):
    if name in SETUPS:
        SETUPS[name](engines)
    for engine in engines:
        engine.ask('(progn (parity-probe-mark-messages) t)')
    for s in steps:
        record = {'step': s['name'], 'observed': {}}
        entry['steps'].append(record)
        for engine in engines:
            engine.keys(*s['keys'])
            ready(engine, s['ready'], s['name'])
            if s.get('then'):
                engine.keys(*s['then'])
                ready(engine, s['then_ready'], f'{s["name"]} then')
            time.sleep(.3)
            record['observed'][engine.name] = normalise(engine.ask(OBSERVE), engine)
            backtrace = engine.ask(BACKTRACE_TEXT)
            if backtrace:
                record.setdefault('backtrace', {})[engine.name] = backtrace
        gnu, romacs = record['observed']['gnu'], record['observed']['romacs']
        if s.get('recency'):
            gnu, romacs = recency_free(gnu), recency_free(romacs)
        record['differences'] = parity_live.differences(gnu, romacs)


def run_quit(engines, entry):
    """C-x C-c over a modified file, then y; M-: (kill-emacs 3) in a second pair."""
    for engine in engines:
        notes = engine.root / 'work/notes.txt'
        notes.write_text('first\n')
        engine.ask(f'(progn (find-file {lisp_string(str(notes))}) (goto-char (point-max)) t)')
        engine.keys(T('second'))
        ready(engine, '(buffer-modified-p)', 'modified')
    record = {'step': 'save-prompt', 'observed': {}}
    entry['steps'].append(record)
    for engine in engines:
        engine.keys(C('x'), C('c'))
        ready(engine, '(string-prefix-p "Save file" (or (minibuffer-prompt) ""))', 'save prompt')
        record['observed'][engine.name] = normalise(engine.ask('(substring-no-properties (minibuffer-prompt))'), engine)
    record['differences'] = parity_live.differences(record['observed']['gnu'], record['observed']['romacs'])
    record = {'step': 'save-and-exit', 'observed': {}}
    entry['steps'].append(record)
    for engine in engines:
        engine.keys(T('y'))
        wait(lambda: engine.process.poll() is not None, engine.timeout, f'{engine.name} exit')
        recorded = engine.probe_dir / 'record.eld'
        record['observed'][engine.name] = {
            'status': engine.process.returncode,
            'file': (engine.root / 'work/notes.txt').read_text(),
            'record': recorded.read_text() if recorded.exists() else None}
    record['differences'] = parity_live.differences(record['observed']['gnu'], record['observed']['romacs'])


def run_autosave(engines, entry):
    """read_char's auto-saves of a visited file: after more than
    `auto-save-interval' input events (at least 20), and after
    `auto-save-timeout' idle seconds (scaled by buffer size: four quarters
    for a small file).  Each must create #FILE# beside FILE."""
    # Keys typed 150 ms apart, so no input is pending at each read (GNU
    # defers the auto-save while some is); the count case's idle timeout is
    # out of reach.
    cases = (('count', 'counted.txt', ['-d', '150', 'x' * 25], 300, 0), ('idle', 'idle.txt', T('yz'), 2, 2))
    for name, file, keys, timeout, idle in cases:
        record = {'step': name, 'observed': {}}
        entry['steps'].append(record)
        for engine in engines:
            engine.ask(f'(progn (setq auto-save-interval 20 auto-save-timeout {timeout}) t)')
            path = engine.root / 'work' / file
            path.write_text('text\n')
            # Through find-file, then auto-save-mode, which GNU's visit turns on itself.
            engine.ask(f'(progn (find-file {lisp_string(str(path))}) (auto-save-mode 1) (goto-char (point-max)) t)')
            auto_save = engine.root / 'work' / f'#{file}#'
            engine.keys(keys)
            ready(engine, f'(string-suffix-p {lisp_string(keys[-1])} (buffer-string))', f'{name} typed')
            try:
                wait(auto_save.exists, idle + 10, f'{engine.name} {auto_save.name}', engine.process)
                created = True
            except TimeoutError:
                created = False
            record['observed'][engine.name] = {'auto-save-file': created,
                                               'contents': auto_save.read_text() if created else None}
        record['differences'] = parity_live.differences(record['observed']['gnu'], record['observed']['romacs'])


def run_sigterm(engines, entry):
    """SIGTERM over a modified file buffer: terminate_due_to_signal runs
    (kill-emacs 15), whose hook and auto-save run before the exit."""
    record = {'step': 'sigterm', 'observed': {}}
    entry['steps'].append(record)
    for engine in engines:
        path = engine.root / 'work' / 'unsaved.txt'
        path.write_text('kept\n')
        engine.ask(f'(progn (find-file {lisp_string(str(path))}) (auto-save-mode 1) (goto-char (point-max)) t)')
        engine.keys(T('new'))
        ready(engine, '(string-suffix-p "new" (buffer-string))', 'typed')
        engine.process.send_signal(parity_live.signal.SIGTERM)
        wait(lambda: engine.process.poll() is not None, engine.timeout, f'{engine.name} exit')
        auto_save = engine.root / 'work' / '#unsaved.txt#'
        recorded = engine.probe_dir / 'record.eld'
        record['observed'][engine.name] = {
            'status': engine.process.returncode, 'file': path.read_text(),
            'auto-save': auto_save.read_text() if auto_save.exists() else None,
            'record': recorded.read_text() if recorded.exists() else None}
    record['differences'] = parity_live.differences(record['observed']['gnu'], record['observed']['romacs'])


def run_status(engines, entry):
    record = {'step': 'kill-emacs-3', 'observed': {}}
    entry['steps'].append(record)
    for engine in engines:
        engine.keys(M(':'), T('(kill-emacs 3)'))
        ready(engine, EVALUATING(lisp_string('(kill-emacs 3)')), 'kill-emacs typed')
        engine.keys(K('Return'))
        wait(lambda: engine.process.poll() is not None, engine.timeout, f'{engine.name} exit')
        record['observed'][engine.name] = {'status': engine.process.returncode}
    record['differences'] = parity_live.differences(record['observed']['gnu'], record['observed']['romacs'])


RUNNERS = {'autosave': run_autosave, 'sigterm': run_sigterm, 'errors': None, 'macros': None, 'kill-buffer': None, 'debugger': None, 'quit': run_quit, 'status': run_status}


def main():
    import argparse
    import hashlib
    import shutil
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=parity_live.os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=parity_live.os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append', choices=list(RUNNERS))
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--keep', action='store_true')
    args = parser.parse_args()
    parity_live.GNU = args.gnu
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'romacs_sha256': hashlib.sha256(Path(args.romacs).read_bytes()).hexdigest(),
              'gnu': args.gnu, 'workflows': []}
    for name in args.workflow or list(RUNNERS):
        entry = {'workflow': name, 'steps': [], 'status': 'FAIL'}
        report['workflows'].append(entry)
        engines = []
        try:
            for engine in ('gnu', 'romacs'):
                engines.append(launch(engine, Path(args.work), args))
            if RUNNERS[name]:
                RUNNERS[name](engines, entry)
            else:
                run_steps(engines, WORKFLOWS[name], entry, name)
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
              json.dumps([s for s in entry['steps'] if s.get('differences', True)], ensure_ascii=False)[:3000], flush=True)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    return 0 if all(w['status'] == 'PASS' for w in report['workflows']) else 1


if __name__ == '__main__':
    sys.exit(main())
