#!/usr/bin/env python3
"""`romacs-live-update' in a live `romacs-vm-gui --loadup' session: Lisp
changed in a worktree reaches the running session, keymaps, options and
the ledger right.

The session runs an install-shaped build in its own private headless labwc,
HOME, XDG and TMPDIR (parity_live.py): a runtime root holding this tree's
lisp/, etc/ and assets/ and a build.json naming the commit they are, made
in a clone of the tree, the source.  Its init loads a fixture library,
lisp/live-journey.el, opens buffers in its mode and in a mode derived from
it, puts its map on text in a third buffer as a `keymap' property, binds
a key of its own in the map and sets one of its options.  The harness then
edits the clone (the fixture's map and options, and a function added to
lisp/emacs-lisp/subr-x.el, preloaded in the session's world) and types
M-x romacs-live-update with the clone's directory.  The update's survey
and the keymaps' library defaults run in worker VMs.

It checks: the new key runs from a real key press in the mode buffer, and
is bound in the derived mode's buffer and at the propertized text; the old
key is gone; the user's key stays; the untouched option has its new
default and the one the user set keeps its value; the preloaded file's new
function is there; the ledger names both files at their blobs and the mode
line shows its mark.

Then M-x romacs-restart: the successor puts the ledger's files back from
its image before the buffers (the same blobs, the new key, the probe, the
mark).  Then M-x romacs-session-reset onto this build (its executable,
its runtime kept): no ledger, the build's code and key, no mark,
no kill ring, the unsaved file buffer carried and the saved one not.

Worlds and compiled Lisp stay under the private
root (--world-store).  A gate build has no compile world (only an
install's whole-tree compile makes one), so the compiled-Lisp store's
identity is not checked here.

usage: live_update_live.py --binary BIN --output DIR [--work DIR]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

import parity_live
from parity_live import C, K, M, T, Engine, lisp_string, plist, private_tree, wait
from restart_live import restart, successor

TREE = Path(__file__).resolve().parents[3]

FIXTURE = ''';;; live-journey.el --- A live update's fixture  -*- lexical-binding: t -*-
(defgroup live-journey nil "The live update journey's fixture." :group 'lisp)
(defcustom live-journey-limit {limit} "Left alone by the user." :type 'integer)
(defcustom live-journey-chosen {chosen} "Set by the user." :type 'integer)
(defvar live-journey-pressed nil "The last command of the fixture's map that ran.")
(defun live-journey-a () "Key a." (interactive) (setq live-journey-pressed 'a))
(defun live-journey-b () "Key b." (interactive) (setq live-journey-pressed 'b))
(defvar live-journey-mode-map
  (let ((map (make-sparse-keymap))) (define-key map (kbd "{key}") #'live-journey-{command}) map))
(define-derived-mode live-journey-mode special-mode "Journey")
(define-derived-mode live-journey-child-mode live-journey-mode "Journey child")
(provide 'live-journey)
'''

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(load {probe} nil t)
(require 'live-journey)
(define-key live-journey-mode-map (kbd "C-c u") #'undo)
(setopt live-journey-chosen 5)
(with-current-buffer (get-buffer-create "journey") (live-journey-mode))
(with-current-buffer (get-buffer-create "journey-child") (live-journey-child-mode))
(with-current-buffer (get-buffer-create "journey-text")
  (insert (propertize "old chat text" 'keymap live-journey-mode-map)) (goto-char 3))
(switch-to-buffer "journey")
(with-temp-file (expand-file-name "pid" parity-probe-dir) (insert (number-to-string (emacs-pid))))
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''

PROBE_FN = '(defun subr-x-live-journey-probe () "Added by the journey." 42)\n'

OBSERVE = '''(list
 :mode (with-current-buffer "journey" (list (key-binding (kbd "C-c b")) (key-binding (kbd "C-c a")) (key-binding (kbd "C-c u"))))
 :child (with-current-buffer "journey-child" (list (key-binding (kbd "C-c b")) (key-binding (kbd "q"))))
 :text (with-current-buffer "journey-text" (key-binding (kbd "C-c b")))
 :options (list live-journey-limit live-journey-chosen)
 :probe (and (fboundp 'subr-x-live-journey-probe) (funcall 'subr-x-live-journey-probe))
 :subr-x (and (cl-find-if (lambda (e) (and (stringp (car e)) (string-suffix-p "/emacs-lisp/subr-x.el" (car e)))) load-history) t))'''


def git_env(private):
    """Git in PRIVATE's HOME and XDG, with no ambient repository, index,
    system or user config: the fixture's own repository only, and no hook
    or attribute of the user's applies."""
    env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
    env.update({k: v for k, v in private.items() if k == 'HOME' or k.startswith('XDG_')})
    env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
    return env


def git(cwd, private, *args):
    return subprocess.run(['git', '-c', 'user.name=Romacs journey', '-c', 'user.email=journey@example.invalid',
                           '-c', 'commit.gpgsign=false', '-c', 'core.hooksPath=/dev/null', *args], cwd=cwd, check=True,
                          capture_output=True, text=True, env=git_env(private)).stdout.strip()


def fixture(**kw):
    values = dict(limit=10, chosen=1, key='C-c a', command='a')
    values.update(kw)
    return FIXTURE.format(**values)


def build_and_source(root, env):
    """A clone of this tree with its working Lisp committed and the fixture
    added (the source), and a runtime root of that commit (the build)."""
    source = root / 'source'
    subprocess.run(['git', '-c', 'core.hooksPath=/dev/null', 'clone', '-q', '--shared', str(TREE), str(source)],
                   check=True, capture_output=True, env=git_env(env))
    for part in ('lisp', 'etc', 'assets'):
        subprocess.run(['rsync', '-a', '--delete', f'{TREE / part}/', f'{source / part}/'], check=True)
    (source / 'lisp/live-journey.el').write_text(fixture())
    git(source, env, 'add', '-A', 'lisp', 'etc', 'assets')
    git(source, env, 'commit', '-q', '--allow-empty', '-m', 'journey base')
    commit = git(source, env, 'rev-parse', 'HEAD')
    build = root / 'build'
    build.mkdir()
    for part in ('lisp', 'etc', 'assets'):
        subprocess.run(['cp', '-a', '--reflink=auto', str(source / part), str(build / part)], check=True)
    (build / 'build.json').write_text(json.dumps({'commit': commit}))
    return source, build, commit



def ready(engine, predicate, what, timeout=None):
    wait(lambda: plist(engine.ask(f'(parity-probe-ready (lambda () {predicate}))')).get('ready'),
         timeout or engine.timeout, f'{engine.name} {what}', engine.process, .05)


def capture(engine, path):
    done = subprocess.run([f'{engine.tools}/grim', str(path)], env=engine.env, capture_output=True, timeout=30)
    return path.name if done.returncode == 0 else None


def run(args, output, entry):
    """The journey, filling ENTRY as it goes: what it saw stays when it fails."""
    root, home, env = private_tree('romacs', Path(args.work), args.tools, prefix='live-update-')
    entry['root'] = str(root)
    source, build, commit = build_and_source(root, env)
    env.update(ROMACS_RUNTIME_ROOT=str(build), ROMACS_LISP_DIR=str(build / 'lisp'),
               ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
    (home / '.config/romacs').mkdir()
    (home / '.config/romacs/init.el').write_text(INIT.format(probe=lisp_string(str(parity_live.PROBE))))
    worlds = ['--world-store', str(root / 'worlds')]
    started = time.time()
    done = subprocess.run([args.romacs, '--loadup', *worlds, '--world-build'], env=env, cwd=root / 'work',
                          stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=900)
    entry['world_seconds'] = round(time.time() - started, 3) if done.returncode == 0 else None
    engine = Engine('romacs', [args.romacs, '--loadup', *worlds], root, None, env, args.timeout)
    try:
        engine.start(args.tools)
        ready(engine, '(eq (frame-focus-state) t)', 'focus')
        ready(engine, '(get-buffer "journey-text")', 'the init')
        before = plist(engine.ask(OBSERVE))
        entry['before'] = before
        (source / 'lisp/live-journey.el').write_text(fixture(limit=20, chosen=2, key='C-c b', command='b'))
        with (source / 'lisp/emacs-lisp/subr-x.el').open('a') as subr:
            subr.write(PROBE_FN)
        engine.keys(M('x'))
        ready(engine, '(minibufferp)', 'M-x prompt')
        engine.keys(T('romacs-live-update'))
        ready(engine, '(equal (minibuffer-contents-no-properties) "romacs-live-update")', 'the command typed')
        engine.keys(K('Return'))
        ready(engine, '(string-match-p "Live update from" (or (minibuffer-prompt) ""))', 'the source prompt')
        engine.keys(T(str(source)), K('Return'))
        started = time.time()
        wait(lambda: engine.ask('(if (and romacs-live-last-report (not romacs-live--running)) 1 0)') == 1,
             engine.timeout, 'the update', engine.process, .05)
        entry['update_seconds'] = round(time.time() - started, 3)
        report = plist(engine.ask('(list :summary (romacs-live-summary romacs-live-last-report)'
                                  ' :refusals (plist-get romacs-live-last-report :refusals)'
                                  ' :failed (plist-get romacs-live-last-report :failed)'
                                  ' :rows (mapcar (lambda (r) (list (plist-get r :path) (plist-get r :action) (plist-get r :ms)'
                                  ' (mapcar (lambda (k) (list (plist-get k :variable) (plist-get k :unchecked)'
                                  ' (mapcar #\'key-description (plist-get k :user)))) (plist-get r :keymaps))))'
                                  ' (plist-get romacs-live-last-report :rows)))'))
        entry['report'] = report
        after = plist(engine.ask(OBSERVE))
        entry['after'] = after
        ledger = engine.ask('(mapcar (lambda (e) (list (plist-get e :path) (plist-get e :state) (plist-get e :new)))'
                            ' (plist-get romacs-live-ledger :entries))')
        entry['ledger'] = ledger
        entry['mode_line'] = engine.ask('(let ((w (get-buffer-window "journey"))) (substring-no-properties '
                                        '(format-mode-line (buffer-local-value (quote mode-line-format) (window-buffer w)) nil w)))')
        blobs = {path: git(source, env, 'hash-object', path) for path in ('lisp/live-journey.el', 'lisp/emacs-lisp/subr-x.el')}
        # A real key press runs the new command in the mode's buffer.
        engine.ask('(progn (setq live-journey-pressed nil) (switch-to-buffer "journey") t)')
        ready(engine, '(equal (buffer-name) "journey")', 'the journey buffer')
        engine.keys(C('c'), T('b'))
        ready(engine, '(eq live-journey-pressed (quote b))', 'C-c b ran', timeout=30)
        engine.ask('(progn (romacs-live-ledger) t)')
        ready(engine, '(get-buffer-window "*romacs-live*")', 'the ledger shown')
        time.sleep(.5)
        entry['capture'] = capture(engine, output / 'ledger.png')
        rows = {row[0]: row for row in report['rows'] or []}
        journey_keymaps = rows.get('lisp/live-journey.el', [None, None, None, []])[3]
        checks = {
            'before: the old key': before['mode'][0] is None and before['mode'][1] == 'live-journey-a',
            'before: subr-x loaded, no probe': before['subr-x'] in (True, 't') and before['probe'] is None,
            'no refusal, no failure': not report['refusals'] and report['failed'] is None,
            'both files loaded': rows.get('lisp/live-journey.el', [0, None])[1] == 'load'
                                 and rows.get('lisp/emacs-lisp/subr-x.el', [0, None])[1] == 'load',
            'mode buffer: new key bound, old gone': after['mode'][0] == 'live-journey-b' and after['mode'][1] is None,
            "user's key kept": after['mode'][2] == 'undo',
            "user's keys checked against the library's defaults": any(
                k[0] == 'live-journey-mode-map' and k[1] is None and 'C-c u' in (k[2] or []) for k in journey_keymaps),
            'derived mode: new key and its parent kept': after['child'] == ['live-journey-b', 'quit-window'],
            'propertized text: new key': after['text'] == 'live-journey-b',
            'untouched option: new default; set option kept': after['options'] == [20, 5],
            'preloaded file: new function': after['probe'] == 42,
            'ledger: both files at their blobs': sorted(ledger or []) == sorted(
                [[p, 'loaded', b] for p, b in blobs.items()]),
            'mode line mark': 'Δ2' in (entry['mode_line'] or ''),
        }
        entry['checks'] = checks
        checks.update(restart_and_reset(engine, root, blobs, entry))
        entry['status'] = 'PASS' if all(checks.values()) else 'FAIL'
    finally:
        engine.stop()
    if entry['status'] == 'PASS' and not args.keep:
        shutil.rmtree(root, ignore_errors=True)
    return entry


RESTARTED = '''(list
 :ledger (mapcar (lambda (e) (list (plist-get e :path) (plist-get e :state) (plist-get e :new))) (plist-get (bound-and-true-p romacs-live-ledger) :entries))
 :report (and (bound-and-true-p romacs-live-last-report) (romacs-live-summary romacs-live-last-report))
 :problems (format "%S" (plist-get (bound-and-true-p warm-restart-last-report) :problems))
 :live (featurep 'romacs-live)
 :mode (with-current-buffer "journey" (list (key-binding (kbd "C-c b")) (key-binding (kbd "C-c a"))))
 :probe (and (fboundp 'subr-x-live-journey-probe) (funcall 'subr-x-live-journey-probe))
 :mark (let ((w (get-buffer-window "journey"))) (and w (substring-no-properties (format-mode-line (buffer-local-value (quote mode-line-format) (window-buffer w)) nil w))))
 :kill (car kill-ring)
 :note (and (get-buffer "reset-note.txt") (with-current-buffer "reset-note.txt" (buffer-string)))
 :kept (and (get-buffer "kept.txt") t))'''


def successor_up(engine, pressed, replays):
    """ENGINE on the process that took over, its restore done and, when it
    REPLAYS a ledger, its replay too."""
    pid, seconds = successor(engine, pressed)
    if replays:
        wait(lambda: engine.ask('(if (and (featurep (quote romacs-live)) romacs-live-last-report'
                                ' (not romacs-live--running)) 1 0)') == 1,
             engine.timeout, 'the replay', engine.process, .05)
    engine.ask('(progn (switch-to-buffer "journey") t)')
    ready(engine, '(equal (buffer-name) "journey")', 'the journey buffer')
    return pid, seconds


def restart_and_reset(engine, root, blobs, entry):
    """A restart carries the ledger and the successor puts its files back;
    a reset then restarts onto the build with neither, carrying only the
    unsaved buffer.  The checks."""
    work = root / 'work'
    old_pid = engine.process.pid
    status, pressed, _ = restart(engine)
    new_pid, entry['restart_seconds'] = successor_up(engine, pressed, True)
    restarted = plist(engine.ask(RESTARTED))
    entry['restarted'] = restarted
    # What a reset drops and keeps: a kill, a saved file, an unsaved one.
    (work / 'kept.txt').write_text('kept on disk\n')
    (work / 'reset-note.txt').write_text('on disk\n')
    engine.ask(f'(progn (kill-new "a kill") (find-file-noselect {lisp_string(str(work / "kept.txt"))})'
               f' (with-current-buffer (find-file-noselect {lisp_string(str(work / "reset-note.txt"))})'
               ' (goto-char (point-max)) (insert "typed\\n")) t)')
    (engine.probe_dir / 'alive').unlink()
    engine.keys(M('x'))
    ready(engine, '(minibufferp)', 'M-x prompt')
    engine.keys(T('romacs-session-reset'))
    ready(engine, '(equal (minibuffer-contents-no-properties) "romacs-session-reset")', 'the reset typed')
    engine.keys(K('Return'))
    ready(engine, '(string-match-p "Reset onto" (or (minibuffer-prompt) ""))', 'the reset question')
    engine.keys(T('yes'), K('Return'))
    pressed = time.time()
    wait(lambda: engine.process.poll() is not None, engine.timeout, 'the session to exit for the reset')
    reset_pid, entry['reset_seconds'] = successor_up(engine, pressed, False)
    reset = plist(engine.ask(RESTARTED))
    entry['reset'] = reset
    return {
        'restart: a new process': status == 0 and new_pid != old_pid,
        'restart: the ledger comes back at its blobs': sorted(restarted['ledger'] or []) == sorted(
            [[p, 'loaded', b] for p, b in blobs.items()]),
        'restart: the files are put back': restarted['probe'] == 42 and restarted['mode'] == ['live-journey-b', None],
        'restart: the mark': 'Δ2' in (restarted['mark'] or ''),
        'reset: a new process': reset_pid != new_pid,
        'reset: no ledger, the build\'s code': not reset['ledger'] and reset['probe'] is None
                                               and reset['mode'] == [None, 'live-journey-a'],
        'reset: no mark': 'Δ' not in (reset['mark'] or ''),
        'reset: no kill ring': reset['kill'] is None,
        'reset: the unsaved buffer carried': reset['note'] == 'on disk\ntyped\n',
        'reset: the saved file buffer not carried': reset['kept'] is None,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', '--romacs', dest='romacs', required=True, help='The binary (romacs-vm-gui)')
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--keep', action='store_true', help='Keep the private tree of a passing run')
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    output = Path(args.output)
    output.mkdir(parents=True)
    args.romacs = str(Path(args.romacs).resolve()) if '/' in args.romacs else args.romacs
    entry = {'workflow': 'worktree'}
    try:
        run(args, output, entry)
    except Exception as error:  # noqa: BLE001 - every failure is the run's result
        entry.update(status='ERROR', error=f'{type(error).__name__}: {error}')
    print(entry['status'], entry.get('error', ''), {k: v for k, v in entry.get('checks', {}).items() if not v},
          entry.get('update_seconds'), flush=True)
    (output / 'report.json').write_text(json.dumps(entry, indent=1, ensure_ascii=False) + '\n')
    return 0 if entry['status'] == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
