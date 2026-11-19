#!/usr/bin/env python3
"""How a live `romacs-vm-gui --loadup' session ends, and what it logged.

Each workflow starts a fresh session in its own private headless labwc,
HOME, XDG and TMPDIR (parity_live.py), with ROMACS_USAGE_LOG naming a file
in its tree, as a launcher sets it; keys go through the command loop,
typed by wtype.  With --launcher, the session runs through that external
launcher instead, from a runtime directory linked at --launcher-runtime
(a path under the private HOME); the launcher then owns the stderr log and
the usage log, which it keeps in a directory per session under
--launcher-sessions (another such path), and --sessions, a session
summariser, summarises them.

  kill-emacs  self-insert keys, a command of 300 ms (F5), a command that
              signals with a file name in its message (F6), a command of
              300 ms after reading a name in the minibuffer (F7, counted
              once in its own mode, timed from the read's end), then
              M-: (kill-emacs 3) RET: exit status 3, stderr's last line
              `romacs-exit: {"reason": "kill-emacs", ..., "status": 3}',
              the usage log's session start, init-done and kill, the counts,
              the slow command, the error with <file> for its file name,
              and none of the typed text.
  compositor  the session's compositor killed (SIGKILL): the session ends
              with status 1, stderr saying `romacs-exit' with the reason
              display-lost.
  startup-failed  a session whose GNU Lisp chain has no bootstrap file
              (ROMACS_LISP_DIR, a tree of links without it), so its
              startup fails: the launch screen stays, saying "Startup
              failed" over the step that failed, red, and the error under
              it, until the window is closed (Alt+F4): status 1, stderr
              saying `romacs-exit' with the reason evaluator-error and the
              file.  Always the binary itself, without a world.
  lisp-error-first  romacs-startup.el's first form signals, which startup
              goes on past: the session's first frame shows that error
              (its file named) at once.
  lisp-error-first-world  lisp-error-first restored from a world built
              over the broken file, which keeps its build's errors: the
              same, from the world's.
  lisp-error-first-world-failed  that world restored by a session whose
              launch font is out of range, so it fails before its startup
              steps: its screen shows the world's first error above that.
  lisp-error-first-failed  the same with the bootstrap gone too, so the
              start fails: its screen shows romacs-startup.el's error
              above what failed after it ("Then: ...").
  startup-failed-server  the same as a server whose client asked for its
              window at its start (--serve NAME --serve-frame): the
              window it asked for shows the same screen, and the same
              close ends it.

usage: exit_live.py --romacs BIN (or --binary BIN) --output DIR [--workflow NAME]...
                   [--launcher LAUNCHER --launcher-runtime REL --launcher-sessions REL [--sessions SUMMARISER]]
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

# The GNU chain's first file (lisp/session/): without it no Lisp starts.
BOOTSTRAP = 'romacs-loadup-bootstrap.el'
# Romacs's first startup file (lisp/session/), and what a broken one signals.
STARTUP = 'romacs-startup.el'
BROKEN = 'broken on purpose'
# A launch font size a launch refuses before its startup steps (6..72 points).
BAD_FONT_SIZE = '99'

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(defun ul-slow () (interactive) (let ((end (+ (float-time) 0.3))) (while (< (float-time) end))))
(defun ul-error () (interactive) (error "Cannot read %s" "/home/someone/private-notes.org"))
(defun ul-ask (name) (interactive "sName: ") (ul-slow) (message "Hello %s" name))
(global-set-key [f5] #'ul-slow)
(global-set-key [f7] #'ul-ask)
(global-set-key [f6] #'ul-error)
(load {probe} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''

SECRET_TEXT = 'zqxjv'


def ready(engine, predicate, what):
    wait(lambda: plist(engine.ask(f'(parity-probe-ready (lambda () {predicate}))')).get('ready'),
         engine.timeout, f'{engine.name} {what}', engine.process, .05)


def runtime(root, romacs):
    """A launcher's runtime directory: the binary and this checkout's Lisp."""
    directory = root / 'runtime'
    directory.mkdir()
    (directory / 'romacs-vm-gui').symlink_to(romacs)
    for name in ('lisp', 'etc'):
        (directory / name).symlink_to(parity_live.ROOT / name)
    return directory


def start(args, base, name):
    """A session for workflow NAME: (ENGINE, its stderr log, its usage log)."""
    root, home, env = private_tree('romacs', base, args.tools, prefix=f'exit-{name}-')
    (home / '.config/romacs').mkdir()
    (home / '.config/romacs/init.el').write_text(INIT.format(probe=lisp_string(str(parity_live.PROBE))))
    env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
    if args.launcher:
        directory = runtime(root, args.romacs)
        (home / args.launcher_runtime).parent.mkdir(parents=True, exist_ok=True)
        (home / args.launcher_runtime).symlink_to(directory)
        env.update(ROMACS_RUNTIME_ROOT=str(directory), ROMACS_LISP_DIR=str(directory / 'lisp'))
        argv = [args.launcher]
    else:
        env['ROMACS_USAGE_LOG'] = str(root / 'usage.jsonl')
        argv = [args.romacs, '--loadup']
    prepare_world(args.romacs, env, root / 'work')
    engine = Engine('romacs', argv, root, None, env, args.timeout)
    if args.launcher:
        # The launcher keeps the editor's stderr in the session's directory.
        engine.editor_log = str((home / args.launcher_sessions).relative_to(root) / '*' / 'stderr.log')
    engine.start(args.tools)
    # Keys typed before the window has the keyboard are lost.
    ready(engine, '(eq (frame-focus-state) t)', 'focus')
    if args.launcher:
        session = next((home / args.launcher_sessions).glob('*/'))
        return engine, session / 'stderr.log', session / 'usage.jsonl'
    return engine, root / 'engine.log', root / 'usage.jsonl'


def start_failed(args, base, name, serve=(), breaks=('bootstrap',), world=False):
    """A session for workflow NAME over this checkout's Lisp tree but for
    BREAKS: `bootstrap', the chain's bootstrap file gone, so its startup
    fails; `startup', romacs-startup.el's first form signalling, which
    startup goes on past.  Launched with SERVE's arguments too, and from a
    world it builds first when WORLD.  Nothing waits for an init."""
    root, _home, env = private_tree('romacs', base, args.tools, prefix=f'exit-{name}-')
    lisp = root / 'lisp'
    (lisp / 'session').mkdir(parents=True)
    for entry in (parity_live.ROOT / 'lisp').iterdir():
        if entry.name != 'session':
            (lisp / entry.name).symlink_to(entry)
    for entry in (parity_live.ROOT / 'lisp/session').iterdir():
        if entry.name == STARTUP and 'startup' in breaks:
            (lisp / 'session' / STARTUP).write_text(f'(error "{BROKEN}")\n' + entry.read_text())
        elif entry.name != BOOTSTRAP or 'bootstrap' not in breaks:
            (lisp / 'session' / entry.name).symlink_to(entry)
    env.update(ROMACS_LISP_DIR=str(lisp), ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
    # A binary copied out of its build (a check's) finds its tree by name.
    if 'ROMACS_RUNTIME_ROOT' in os.environ:
        env['ROMACS_RUNTIME_ROOT'] = os.environ['ROMACS_RUNTIME_ROOT']
    if not world:
        env['ROMACS_WORLD'] = 'off'
    engine = Engine('romacs', [args.romacs, '--loadup', *serve], root, None, env, args.timeout)
    try:
        engine.spawn(args.tools)
    except Exception:
        engine.stop()
        raise
    return engine, root / 'engine.log', None


def exit_line(log):
    lines = [line for line in log.read_text(errors='replace').splitlines() if line.startswith('romacs-exit: ')]
    return json.loads(lines[-1].removeprefix('romacs-exit: ')) if lines else None


def ended(engine):
    wait(lambda: engine.process.poll() is not None, engine.timeout, 'session exit')
    return engine.process.returncode


def run_kill_emacs(engine, stderr, usage):
    engine.keys(T(SECRET_TEXT), C('a'))
    ready(engine, f'(string-prefix-p {lisp_string(SECRET_TEXT)} (buffer-string))', 'typed')
    engine.keys(K('F5'))
    engine.keys(K('F6'))
    ready(engine, '(equal (current-message) "Cannot read /home/someone/private-notes.org")', 'the error shown')
    engine.keys(K('F7'))
    ready(engine, '(equal (minibuffer-prompt) "Name: ")', 'the prompt')
    engine.keys(T('ab'))
    ready(engine, '(equal (minibuffer-contents-no-properties) "ab")', 'the name typed')
    time.sleep(1.5)
    engine.keys(K('Return'))
    ready(engine, '(equal (current-message) "Hello ab")', 'the prompt command done')
    engine.keys(M(':'), T('(kill-emacs 3)'))
    ready(engine, f'(equal (minibuffer-contents-no-properties) {lisp_string("(kill-emacs 3)")})', 'kill-emacs typed')
    engine.keys(K('Return'))
    status = ended(engine)
    text = usage.read_text()
    lines = [json.loads(line) for line in text.splitlines()]
    counts = {(r['command'], r['mode']): r['n'] for r in lines if r['kind'] == 'command' and 'n' in r}
    slow = [r for r in lines if r['kind'] == 'command' and 'ms' in r]
    errors = [r for r in lines if r['kind'] == 'error']
    events = [r['event'] for r in lines if r['kind'] == 'session']
    checks = {
        'status 3': status == 3,
        'romacs-exit kill-emacs 3': exit_line(stderr) == {'reason': 'kill-emacs', 'detail': '', 'status': 3},
        'session start, init-done, kill': events == ['start', 'init-done', 'kill'],
        'self-insert counted': counts.get(('self-insert-command', 'lisp-interaction-mode'), 0) >= len(SECRET_TEXT),
        'slow command': any(r['command'] == 'ul-slow' and r['ms'] >= 300 for r in slow),
        'prompt command once, in its mode': counts.get(('ul-ask', 'lisp-interaction-mode')) == 1
            and not any(c == 'ul-ask' and m != 'lisp-interaction-mode' for c, m in counts),
        'prompt command timed from the read': [r['mode'] for r in slow if r['command'] == 'ul-ask'] == ['lisp-interaction-mode']
            and all(300 <= r['ms'] < 1300 for r in slow if r['command'] == 'ul-ask'),
        # A record says more of an error than this (its mode, its backtrace): these, at least.
        'error with <file>': any(r.get('command') == 'ul-error' and r.get('error') == '(error . "<file>")' for r in errors),
        'no typed text or file name': SECRET_TEXT not in text and 'private-notes' not in text,
    }
    return checks, {'status': status, 'exit': exit_line(stderr), 'usage': lines}


def run_compositor(engine, stderr, usage):
    engine.keys(T('x'))
    ready(engine, '(string-prefix-p "x" (buffer-string))', 'typed')
    engine.processes[0].send_signal(signal.SIGKILL)
    status = ended(engine)
    line = exit_line(stderr)
    checks = {'status 1': status == 1, 'romacs-exit display-lost': bool(line) and line['reason'] == 'display-lost'
              and line['status'] == 1}
    return checks, {'status': status, 'exit': line}


def screen_text(engine):
    """What the screen shows, read back (grim, tesseract)."""
    shot = engine.root / 'screen.png'
    subprocess.run([f'{engine.tools}/grim', str(shot)], env=engine.env, check=True, timeout=30, capture_output=True)
    return subprocess.run([f'{engine.tools}/tesseract', str(shot), '-'], capture_output=True, text=True, timeout=60).stdout


def run_startup_failed(engine, stderr, usage):
    wait(engine.startup_failed, engine.timeout, 'the failed startup', engine.process)

    # The window shows once the display has started, which may be after the error.
    def shown():
        text = screen_text(engine)
        return text if 'Startup failed' in text else None
    screen = wait(shown, engine.timeout, 'the failed startup shown', engine.process, 1)
    time.sleep(2)
    held = engine.process.poll() is None
    engine.keys(M('F4'))
    status = ended(engine)
    line = exit_line(stderr)
    checks = {
        'the window outlives the error': held,
        'the screen says at which step the startup failed': 'Starting Lisp' in screen,
        'and why': 'No such file or directory' in screen,
        'closed: status 1': status == 1,
        'romacs-exit evaluator-error naming the file': bool(line) and line['reason'] == 'evaluator-error'
            and line['status'] == 1 and BOOTSTRAP in line['detail'],
    }
    return checks, {'status': status, 'exit': line, 'screen': screen}


def start_world_failed(args, base, name):
    """lisp-error-first-world's world, then a session restored from it
    that fails before its startup steps: a launch font out of range."""
    first, _, _ = start_failed(args, base, f'{name}-build', breaks=('startup',), world=True)
    log = first.root / 'engine.log'
    try:
        wait(lambda: '"diagnostic":"paint"' in log.read_text(errors='replace'), first.timeout * 4,
             "the world's first session", first.process)
    finally:
        first.stop()
    root, _home, env = private_tree('romacs', base, args.tools, prefix=f'exit-{name}-')
    for key in ('HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'ROMACS_LISP_DIR', 'ROMACS_GUI_FONT'):
        env[key] = first.env[key]
    env['ROMACS_GUI_FONT_SIZE'] = BAD_FONT_SIZE
    engine = Engine('romacs', [args.romacs, '--loadup'], root, None, env, args.timeout)
    try:
        engine.spawn(args.tools)
    except Exception:
        engine.stop()
        raise
    return engine, root / 'engine.log', None


def run_first_lisp_error(engine, stderr, usage):
    """The first noted Lisp error is what the user sees first: on a failed
    start's screen, above what failed after it; in a degraded session's
    first frame, at once."""
    log = engine.root / 'engine.log'
    wait(lambda: engine.startup_failed() or '"diagnostic":"paint"' in log.read_text(errors='replace'),
         engine.timeout * 4, 'a failed start or a first paint', engine.process)

    def shown():
        text = ' '.join(screen_text(engine).split())
        return text if BROKEN in text else None
    screen = wait(shown, engine.timeout, 'the first Lisp error shown', engine.process, 1)
    failed = bool(engine.startup_failed())
    # Read without whitespace: a long path wraps anywhere on the screen.
    compact = ''.join(screen.split())
    first, then, symptom = compact.find(STARTUP), compact.lower().find('then:'), compact.find('(void-')
    world = engine.env.get('ROMACS_WORLD') != 'off'
    checks = {
        'restored from its world, when it had one': not world or '"mode":"restored"' in log.read_text(errors='replace'),
        'the first error shown, its file named': first >= 0,
        'before what failed after it': then < 0 or first < then,
        'no symptom shown without its cause': symptom < 0 or 0 <= then < symptom,
        'a failed start shows what failed after it': not failed or then >= 0,
    }
    if engine.env.get('ROMACS_GUI_FONT_SIZE') == BAD_FONT_SIZE:
        checks['it failed, on the launch font'] = failed and 'fontsizemustbe' in compact[max(then, 0):]
    return checks, {'failed': failed, 'screen': screen}


WORKFLOWS = {'kill-emacs': run_kill_emacs, 'compositor': run_compositor, 'startup-failed': run_startup_failed,
             'startup-failed-server': run_startup_failed, 'lisp-error-first': run_first_lisp_error,
             'lisp-error-first-failed': run_first_lisp_error, 'lisp-error-first-world': run_first_lisp_error,
             'lisp-error-first-world-failed': run_first_lisp_error}
STARTS = {'startup-failed': start_failed,
          'startup-failed-server': lambda args, base, name: start_failed(args, base, name, ('--serve', 'exit-live', '--serve-frame')),
          'lisp-error-first': lambda args, base, name: start_failed(args, base, name, breaks=('startup',)),
          'lisp-error-first-failed': lambda args, base, name: start_failed(args, base, name, breaks=('startup', 'bootstrap')),
          'lisp-error-first-world': lambda args, base, name: start_failed(args, base, name, breaks=('startup',), world=True),
          'lisp-error-first-world-failed': start_world_failed}


def main(default=None):
    """Run the workflows named, else DEFAULT's, else every one."""
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', '--binary', dest='romacs', required=True)
    parser.add_argument('--launcher', help='run each session through this launcher')
    parser.add_argument('--launcher-runtime', help="with --launcher: where, under HOME, it finds the build's runtime")
    parser.add_argument('--launcher-sessions', help='with --launcher: where, under HOME, it keeps per-session logs')
    parser.add_argument('--sessions', help='with --launcher: the session summariser')
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append')
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    if args.launcher and not (args.launcher_runtime and args.launcher_sessions):
        parser.error('--launcher needs --launcher-runtime and --launcher-sessions')
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'launcher': args.launcher, 'workflows': []}
    for name in args.workflow or default or list(WORKFLOWS):
        entry = {'workflow': name}
        report['workflows'].append(entry)
        engine = None
        try:
            engine, stderr, usage = STARTS.get(name, start)(args, Path(args.work), name)
            checks, entry['observed'] = WORKFLOWS[name](engine, stderr, usage)
            entry['checks'] = checks
            if args.launcher and args.sessions:
                home = engine.env['HOME']
                entry['sessions'] = subprocess.run([sys.executable, args.sessions], env={**os.environ, 'HOME': home},
                                                   capture_output=True, text=True, timeout=60).stdout
            entry['status'] = 'PASS' if all(checks.values()) else 'FAIL'
        except Exception as error:  # noqa: BLE001 - every failure is the workflow's result
            entry.update(status='ERROR', error=f'{type(error).__name__}: {error}')
        finally:
            if engine:
                engine.stop()
        print(name, entry['status'], entry.get('error', ''), {k: v for k, v in entry.get('checks', {}).items() if not v},
              flush=True)
        if entry.get('sessions'):
            print(entry['sessions'], flush=True)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    return 0 if all(w['status'] == 'PASS' for w in report['workflows']) else 1


if __name__ == '__main__':
    sys.exit(main())
