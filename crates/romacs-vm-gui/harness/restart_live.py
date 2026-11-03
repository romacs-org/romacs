#!/usr/bin/env python3
"""`romacs-restart' in a live `romacs-vm-gui --loadup' session: the session
comes back in a new process.

Each workflow starts a session in its own private headless labwc, HOME,
XDG and TMPDIR (parity_live.py) and drives it with keys through the
command loop (wtype): it visits two files and edits both without saving,
splits the frame into three windows, kills and yanks a line, and runs
M-x romacs-restart.  The session must exit with status 0 and a new
process must come up in the same compositor, restore the image after its
init, and show the same buffers (text, point, modified flag), the same
window layout (buffer, point and start of each window, the selected one),
the kill ring's head and the command history, while the files on disk
still hold their original text.

  same    the new process is the same binary (romacs-restart's default).
  newer   the session runs --older BINARY from --older-runtime DIR (its
          own copy of rlisp, gnu-lisp and etc, as an installed build has),
          and `romacs-restart-command' names --romacs: the new process is
          the newer build with its own runtime.

ROMACS_HARNESS_WLR_ENV (KEY=VALUE ...) puts the compositor on the real
GPU; Romacs then draws with the host's Vulkan.  Each workflow saves a
capture of the frame before the restart and after the restore (grim).

usage: restart_live.py --romacs BIN --output DIR [--workflow NAME]...
                       [--older BIN --older-runtime DIR]
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

import parity_live
from parity_live import C, K, M, T, Engine, lisp_string, plist, prepare_world, private_tree, wait

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
{extra}
(load {probe} nil t)
(with-temp-file (expand-file-name "pid" parity-probe-dir) (insert (number-to-string (emacs-pid))))
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''

A_TEXT = 'alpha one\nalpha two\nalpha three\n'
B_TEXT = ';;; b.el\n(defun b-one () 1)\n(defun b-two () 2)\n'

OBSERVE = '''(list
 :buffers (mapcar (lambda (name)
                    (with-current-buffer name
                      (list name (buffer-substring-no-properties (point-min) (point-max))
                            (and (buffer-modified-p) t) (point) (symbol-name major-mode))))
                  '("a.txt" "b.el"))
 :windows (mapcar (lambda (w) (list (buffer-name (window-buffer w)) (window-point w) (window-start w)
                                    (eq w (selected-window)) (window-left-column w) (window-top-line w)))
                  (window-list nil 'no-minibuf (frame-first-window)))
 :kill (and kill-ring (substring-no-properties (car kill-ring)))
 :history (seq-take extended-command-history 3)
 :exe (expand-file-name invocation-name invocation-directory)
 :frame (list (frame-pixel-width) (frame-pixel-height)))'''


class Pid:
    """A process this harness did not start, known by PID: the successor."""

    def __init__(self, pid):
        self.pid, self.returncode = pid, None

    def poll(self):
        try:
            os.kill(self.pid, 0)
            status = Path(f'/proc/{self.pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
            return 0 if status == 'Z' else None
        except (ProcessLookupError, FileNotFoundError):
            self.returncode = 0
            return 0

    def terminate(self):
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass

    def kill(self):
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    def wait(self, timeout=None):
        try:
            wait(lambda: self.poll() is not None, timeout or 30, f'pid {self.pid} exit')
        except TimeoutError:
            raise subprocess.TimeoutExpired(f'pid {self.pid}', timeout or 30) from None
        return 0


def ready(engine, predicate, what, timeout=None):
    wait(lambda: plist(engine.ask(f'(parity-probe-ready (lambda () {predicate}))')).get('ready'),
         timeout or engine.timeout, f'{engine.name} {what}', engine.process, .05)


def capture(engine, path):
    done = subprocess.run([f'{engine.tools}/grim', str(path)], env=engine.env, capture_output=True, timeout=30)
    return path.name if done.returncode == 0 else None


def start(args, base, name):
    """The session of workflow NAME, and the binary its successor must be."""
    root, home, env = private_tree('romacs', base, args.tools, prefix=f'restart-{name}-')
    wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
    if wlr:
        env.update(wlr)
        env.pop('WGPU_BACKEND', None)
    env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
    extra = ''
    binary = args.romacs
    if name == 'newer':
        binary = args.older
        runtime = Path(args.older_runtime)
        env.update(ROMACS_RUNTIME_ROOT=str(runtime), ROMACS_LISP_DIR=str(runtime / 'gnu-lisp'))
        extra = f'(setq romacs-restart-command (list {lisp_string(args.romacs)} "--loadup"))'
        # The newer build's world, prepared as an installer would, with its
        # own runtime: what the restart command's environment gives it.
        newer_env = {k: v for k, v in env.items() if k not in ('ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR')}
        prepare_world(args.romacs, newer_env, root / 'work')
    (home / '.config/romacs').mkdir()
    (home / '.config/romacs/init.el').write_text(INIT.format(extra=extra, probe=lisp_string(str(parity_live.PROBE))))
    (root / 'work/a.txt').write_text(A_TEXT)
    (root / 'work/b.el').write_text(B_TEXT)
    world = prepare_world(binary, env, root / 'work')
    engine = Engine('romacs', [binary, '--loadup'], root, None, env, args.timeout).start(args.tools)
    ready(engine, '(eq (frame-focus-state) t)', 'focus')
    return engine, args.romacs, world


def visit(engine, path):
    engine.keys(C('x'), C('f'))
    ready(engine, '(minibufferp)', 'find-file prompt')
    engine.keys(C('a'), C('k'), T(str(path)), K('Return'))
    ready(engine, f'(equal (buffer-file-name) {lisp_string(str(path))})', f'{path.name} visited')


def edit(engine, root):
    """Visit, edit, split and kill as a user would; return the observation."""
    work = root / 'work'
    visit(engine, work / 'a.txt')
    engine.keys(M('>'), T('added line'), M('<'), T('top '))
    ready(engine, '(string-prefix-p "top alpha" (buffer-string))', 'a.txt edited')
    engine.keys(C('x'), T('3'), C('x'), T('o'))
    ready(engine, '(= (length (window-list)) 2)', 'split right')
    visit(engine, work / 'b.el')
    engine.keys(M('>'), T(';; tail'), C('a'), C('k'), C('y'))
    ready(engine, '(string-suffix-p ";; tail" (buffer-string))', 'b.el edited')
    engine.keys(C('x'), T('2'))
    ready(engine, '(= (length (window-list)) 3)', 'split below')
    engine.keys(C('x'), T('o'), M('<'), C('n'))
    ready(engine, '(= (line-number-at-pos) 2)', 'point moved in the lower window')
    return plist(engine.ask(OBSERVE))


def restart(engine):
    """Run M-x romacs-restart; answer its question about processes if it asks."""
    asking = engine.ask('(seq-filter (lambda (p) (plist-get p :query)) (plist-get (warm-restart-plan) :processes))')
    (engine.probe_dir / 'alive').unlink()
    engine.keys(M('x'))
    ready(engine, '(minibufferp)', 'M-x prompt')
    engine.keys(T('romacs-restart'), K('Return'))
    if asking:
        ready(engine, '(string-match-p "Restart anyway" (or (minibuffer-prompt) ""))', 'the restart question')
        engine.keys(T('yes'), K('Return'))
    pressed = time.time()
    wait(lambda: engine.process.poll() is not None, engine.timeout, 'the session to exit')
    return engine.process.returncode, pressed, asking


def successor(engine, pressed):
    """Point ENGINE at the process that took over, once it has restored."""
    wait(lambda: (engine.probe_dir / 'alive').exists(), engine.timeout * 2, 'the new process init')
    pid = int((engine.probe_dir / 'pid').read_text())
    engine.process = Pid(pid)
    engine.processes.append(engine.process)
    ready(engine, '(and warm-restart-last-report t)', 'the restore')
    ready(engine, '(eq (frame-focus-state) t)', 'focus in the new process')
    return pid, round(time.time() - pressed, 3)


def run(args, base, name, output):
    engine, expected_exe, world = start(args, base, name)
    entry = {'world_seconds': world}
    try:
        root = engine.root
        before = edit(engine, root)
        entry['before'] = before
        entry['capture_before'] = capture(engine, output / f'{name}-before.png')
        old_pid = engine.process.pid
        status, pressed, asking = restart(engine)
        new_pid, seconds = successor(engine, pressed)
        time.sleep(.5)
        entry['capture_after'] = capture(engine, output / f'{name}-after.png')
        after = plist(engine.ask(OBSERVE))
        report = plist(engine.ask('(list :buffers (plist-get warm-restart-last-report :buffers)'
                                  ' :seconds (plist-get warm-restart-last-report :seconds)'
                                  ' :problems (length (plist-get warm-restart-last-report :problems))'
                                  ' :lost (mapcar (lambda (e) (format "%S" e)) (plist-get warm-restart-last-report :lost))'
                                  ' :error (plist-get warm-restart-last-report :error))'))
        state = root / 'home/.local/state/romacs/warm-restart'
        entry.update(after=after, report=report, status=status, asked=asking, old_pid=old_pid, new_pid=new_pid,
                     restart_seconds=seconds, images=sorted(p.name for p in state.iterdir()) if state.exists() else [])
        same = lambda key: before[key] == after[key]
        checks = {
            'session exited 0': status == 0,
            'a new process': new_pid != old_pid,
            'buffers: text, modified, point, mode': same('buffers'),
            'unsaved: both modified': all(b[2] for b in after['buffers']),
            'windows: buffer, point, start, selected': [w[:4] for w in before['windows']] == [w[:4] for w in after['windows']],
            # GNU's window-state-put over Romacs's windows places dividers
            # within a line or column of the captured layout, restart or not.
            'windows: place within a line or column': len(before['windows']) == len(after['windows']) and all(
                abs(b[4] - a[4]) <= 1 and abs(b[5] - a[5]) <= 1 for b, a in zip(before['windows'], after['windows'])),
            'three windows': len(after['windows']) == 3,
            'kill ring head': same('kill') and before['kill'] is not None,
            'command history has romacs-restart': 'romacs-restart' in (after['history'] or []),
            'files on disk untouched': (root / 'work/a.txt').read_text() == A_TEXT
                                       and (root / 'work/b.el').read_text() == B_TEXT,
            'restore without problems': report['problems'] == 0 and report['error'] is None,
            'image consumed': 'restored.image' in entry['images'],
            'the expected build': after['exe'] == str(Path(expected_exe).resolve()) or after['exe'] == expected_exe,
        }
        if name == 'newer':
            checks['the older build wrote it'] = before['exe'] in (args.older, str(Path(args.older).resolve()))
        entry['checks'] = checks
        entry['status'] = 'PASS' if all(checks.values()) else 'FAIL'
    finally:
        engine.stop()
    return entry


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--older', help='newer: the build the session starts in')
    parser.add_argument('--older-runtime', help="newer: that build's runtime directory (rlisp, gnu-lisp, etc)")
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append', choices=['same', 'newer'])
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    workflows = args.workflow or (['same', 'newer'] if args.older else ['same'])
    if 'newer' in workflows and not (args.older and args.older_runtime):
        parser.error('newer needs --older and --older-runtime')
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'older': args.older, 'workflows': []}
    for name in workflows:
        entry = {'workflow': name}
        report['workflows'].append(entry)
        try:
            entry.update(run(args, Path(args.work), name, output))
        except Exception as error:  # noqa: BLE001 - every failure is the workflow's result
            entry.update(status='ERROR', error=f'{type(error).__name__}: {error}')
        print(name, entry['status'], entry.get('error', ''),
              {k: v for k, v in entry.get('checks', {}).items() if not v}, entry.get('restart_seconds'), flush=True)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    return 0 if all(w['status'] == 'PASS' for w in report['workflows']) else 1


if __name__ == '__main__':
    sys.exit(main())
