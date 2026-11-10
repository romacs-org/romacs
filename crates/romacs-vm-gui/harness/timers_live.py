#!/usr/bin/env python3
"""Lisp timers in a native --loadup session, in a private headless compositor.

A private init.el installs probes that append to a state file: an ordinary
timer due 2s after init, at 3s gnosis-feed's dwell probe (a timed read that a
timer's queued input must wake), and once that returns a repeating 1s idle
timer that records `current-idle-time' and messages.  The session then sits
idle (the idle probe must fire once, at >= 1s of idleness, the ordinary one
once), and its CPU time over an idle window is read from /proc.  Real keys
then run M-x gnosis-dashboard, whose statistics arrive by `run-with-timer'
and whose link audit follows by `run-with-idle-timer': "Loading
statistics..." must give way to them, and the command's end must start a new idle period (one more
idle probe after it).  The M-x prompt's own wait for a key is idle too, as
GNU's read_char is: a probe may fire there, inside the minibuffer, however
long the keys take to come, but never while a command runs.  That session
runs with --trace-scene, whose scene dump on every repaint the checks read;
idle CPU is measured in a second session with the same init and keys but no
dumps, once the evaluator is quiet after startup and again after the
dashboard.  A third session's init cancels every timer and leaves one that
messages: the paint that shows it, submitted with no input and no timer
after it, must retire at once, woken by its render, not at the render stall.

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY; DISPLAY is never set.  labwc, wlr-randr, wtype and grim come
from --tools (the gate's headless tools) or PATH.
"""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live

INIT = '''(setq gnosis-dir {gnosis})
(defvar timers-live-idle 0)
;; Commands begun so far: a key ends an idle period by running one, so a
;; probe's count names its idle period.
(defvar timers-live-commands 0)
(add-hook 'pre-command-hook (lambda () (setq timers-live-commands (1+ timers-live-commands))))
(defun timers-live-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(timers-live-record '(init))
;; What the host hands the display for each echo-area message (the echo
;; line, or a toast in romacs-complete-mode), with `current-message' then.
(add-hook 'romacs-echo-area-functions
          (lambda (text) (when text (timers-live-record (list 'echo text (current-message))))))
(run-with-timer 2 nil (lambda () (timers-live-record (list 'ordinary (and (current-idle-time) t)))))
(defun timers-live-idle-probe ()
  (setq timers-live-idle (1+ timers-live-idle))
  (timers-live-record (list 'idle timers-live-idle (float-time (current-idle-time)) (minibuffer-depth)
                            timers-live-commands))
  (message "Idle probe %d" timers-live-idle))
;; The idle probe starts once the dwell probe's read has returned: inside
;; that read its message could be wiped by the read's own event within one
;; timer pass, never painted (DIVERGENCES.md VM2-REDISPLAY1).  Registered
;; while idle, it fires at 1s of this idle period (timer.el's DONT-WAIT).
(run-with-timer 3 nil (lambda () (require 'gnosis-feed) (timers-live-record (list 'dwell (gnosis-feed--dwell-supported-p)))
                        (run-with-idle-timer 1 t #'timers-live-idle-probe)))
(add-hook 'post-command-hook
          (lambda () (when (eq this-command 'gnosis-dashboard) (timers-live-record '(dashboard-done)))))
'''


def cpu_ticks(pid):
    """utime + stime of PID, and per thread name its ticks and context switches."""
    def ticks(stat):
        fields = stat.read_text().rsplit(')', 1)[1].split()
        return int(fields[11]) + int(fields[12])
    threads = {}
    for task in Path(f'/proc/{pid}/task').iterdir():
        try:
            name = (task / 'comm').read_text().strip()
            status = dict(line.split(':\t', 1) for line in (task / 'status').read_text().splitlines() if ':\t' in line)
            switches = int(status['voluntary_ctxt_switches']) + int(status['nonvoluntary_ctxt_switches'])
            old = threads.get(name, (0, 0))
            threads[name] = (old[0] + ticks(task / 'stat'), old[1] + switches)
        except FileNotFoundError:
            pass
    return ticks(Path(f'/proc/{pid}/stat')), threads


def idle_cpu(pid, seconds):
    """Percent of one core PID used over SECONDS, in total, and per busy
    thread name its percent and wakeups (context switches) per second."""
    hz = os.sysconf('SC_CLK_TCK')
    total0, threads0 = cpu_ticks(pid)
    start = time.monotonic()
    time.sleep(seconds)
    total1, threads1 = cpu_ticks(pid)
    elapsed = time.monotonic() - start
    pct = lambda ticks: round(100 * ticks / hz / elapsed, 2)
    busy = {}
    for name, (ticks, switches) in threads1.items():
        old_ticks, old_switches = threads0.get(name, (0, 0))
        if (ticks, switches) != (old_ticks, old_switches):
            busy[name] = {'percent': pct(ticks - old_ticks),
                          'wakeups_per_second': round((switches - old_switches) / elapsed, 1)}
    return {'seconds': round(elapsed, 2), 'percent_of_core': pct(total1 - total0), 'threads': busy}


RETIRE_INIT = '''(blink-cursor-mode -1)
(run-with-timer 4 nil (lambda ()
  (mapc (function cancel-timer) (append timer-list timer-idle-list nil))
  (run-with-timer 1 nil (lambda () (message "retire probe")))))
'''
RETIRE_STALL = 30


def session_env(root, args, init=None):
    """A private HOME/XDG tree under ROOT, with init.el (INIT, or the
    timers' probes) installed: (env, state, config)."""
    home, work, config = [root / name for name in ('home', 'work', 'config')]
    for directory in (home, work, config, work / 'gnosis'):
        directory.mkdir()
    state = work / 'timers-state.el'
    (home / 'config/romacs').mkdir(parents=True)
    (home / 'config/romacs/init.el').write_text(
        init or INIT.format(state=json.dumps(str(state)), gnosis=json.dumps(str(work / 'gnosis') + '/')))
    (config / 'rc.xml').write_text(live.RC_XML)
    path = os.environ.get('PATH', '/usr/bin:/bin')
    env = {'PATH': f'{args.tools}:{path}' if args.tools else path,
           'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
           'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
           'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
           'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
           'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
           'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
           'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '14'}
    for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_WORLD', 'ROMACS_GATE_FONTCONFIG'):
        if key in os.environ:
            env[key] = os.environ[key]
    if 'ROMACS_GATE_FONTCONFIG' in env:
        env['FONTCONFIG_FILE'] = env['ROMACS_GATE_FONTCONFIG']
    return env, state, work, config


@contextlib.contextmanager
def session(binary, args, output, name, report, trace, init=None, extra_env=None):
    """A private compositor and `BINARY --loadup' (with --trace-scene when
    TRACE) on INIT and EXTRA_ENV, logging to OUTPUT/NAME.stderr: yields
    (env, state, process, log)."""
    compositor = keyboard = process = None
    try:
        with tempfile.TemporaryDirectory(prefix='timers-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            env, state, work, config = session_env(root, args, init)
            env.update(extra_env or {})
            log_path = output / f'{name}.stderr'
            with (output / f'{name}-compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                socket = root / env['WAYLAND_DISPLAY']
                assert 'DISPLAY' not in env and socket.is_socket() and socket.parent == root
                report.setdefault('display', {})[name] = {'wayland_socket': str(socket), 'compositor_pid': compositor.pid}
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)
                # A held key gives the seat a keyboard before the client starts.
                keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(binary), '--loadup'] + (['--trace-scene'] if trace else []), env=env,
                                           cwd=work, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                child_env = Path(f'/proc/{process.pid}/environ').read_bytes().split(b'\0')
                report['display'][name]['client_wayland_display'] = next(
                    (e.decode() for e in child_env if e.startswith(b'WAYLAND_DISPLAY=')), None)
                report['display'][name]['client_has_display'] = any(e.startswith(b'DISPLAY=') for e in child_env)
                yield env, state, process, log_path
    finally:
        live.stop(process)
        live.stop(keyboard)
        live.stop(compositor)


def keys(env, *parts):
    subprocess.run(['wtype', '-d', '40', *parts], env=env, check=True, timeout=20)
    time.sleep(.15)


def records(state):
    return [line for line in state.read_text().splitlines()] if state.exists() else []


def idle_probes(lines):
    """Each idle probe in LINES: (count, idleness, minibuffer depth,
    commands begun before it, which names its idle period)."""
    return [(int(n), float(idleness), int(depth), int(commands.rstrip(')')))
            for n, idleness, depth, commands in (line.split()[1:] for line in lines if line.startswith('(idle '))]


def after_dashboard(lines):
    """LINES from the end of M-x gnosis-dashboard on, or None before it."""
    return lines[lines.index('(dashboard-done)') + 1:] if '(dashboard-done)' in lines else None


def evaluator_ticks(pid):
    """CPU ticks of PID's evaluator thread."""
    for task in Path(f'/proc/{pid}/task').iterdir():
        if (task / 'comm').read_text().startswith('romacs-evaluat'):
            return int((task / 'stat').read_text().rsplit(')', 1)[1].split()[11])
    raise RuntimeError('no evaluator thread')


def quiet(process, seconds, timeout):
    """Wait until the evaluator has used no CPU for SECONDS."""
    end = time.monotonic() + timeout
    last, since = evaluator_ticks(process.pid), time.monotonic()
    while time.monotonic() - since < seconds:
        if time.monotonic() > end or process.poll() is not None:
            raise RuntimeError('the evaluator never went quiet')
        time.sleep(.1)
        if (now := evaluator_ticks(process.pid)) != last:
            last, since = now, time.monotonic()


def scenes(text):
    """Every complete submitted scene in TEXT, oldest first."""
    result = []
    for line in text.split('\n'):
        if line.startswith('{') and '"submitted-scene"' in line:
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if record.get('diagnostic') == 'submitted-scene':
                result.append(record)
    return result


def checked(binary, args, output, report):
    """The timer checks, over --trace-scene's scene dumps."""
    with session(binary, args, output, 'binary', report, True) as (env, state, process, log_path):
        def rows():
            return ff.current_rows(log_path.read_text(errors='replace'))

        def check(name, ok, detail):
            report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
            if not ok:
                report['failures'].append(name)

        def capture(name):
            report['captures'].append({'name': name, 'rows': rows()})
            fio.shot(output / f'{name}.png', env, report)

        fio.wait(lambda: '(init)' in records(state), args.timeout, 'init.el after --loadup', process)
        fio.wait(rows, args.timeout, 'first frame', process)
        # Idle probe: fires once at >= 1s of idleness; the ordinary timer once.
        # Not fio.wait: a binary without timers still reaches its checks.
        # Each wait ends at its event; the bound only ends a failing one,
        # however late a loaded host runs the timers.
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline and not (any(r.startswith('(idle 1 ') for r in records(state))
                                                   and any(r.startswith('(ordinary') for r in records(state))):
            time.sleep(.1)
        time.sleep(.5)
        capture('idle-probe')
        first = records(state)
        probes = idle_probes(first)
        check('idle probe fired once with >= 1s idleness', len(probes) == 1 and probes[0][1] >= 1.0, first)
        check('ordinary timer fired once, while idle', first.count('(ordinary t)') == 1, first)
        fio.wait(lambda: any(r.startswith('(dwell') for r in records(state)), args.timeout, 'dwell probe', process)
        check("a timer's queued input wakes a read (gnosis-feed--dwell-supported-p)",
              '(dwell t)' in records(state), records(state))
        # The display received it (whatever draws it, the echo line or a
        # toast) with `current-message' it, and a frame submitted before any
        # key showed it: recorded conditions, not a frame that may already
        # have changed.
        shown = '(echo "Idle probe 1" "Idle probe 1")'
        def drawn():
            return any('Idle probe 1' in row for scene in scenes(log_path.read_text(errors='replace'))
                       for row in fio.scene_rows(scene))
        try:
            fio.wait(lambda: shown in records(state) and drawn(), args.timeout, 'idle probe shown', process)
        except Exception:
            pass
        check('echo area shows the idle probe', shown in records(state),
              [r for r in records(state) if r.startswith('(echo')])
        check('a frame shows the idle probe before any key', drawn(), rows())
        time.sleep(args.cpu_window)
        check('no more idle probes without input', len(idle_probes(records(state))) == 1, records(state))

        keys(env, '-M', 'alt', 'x', '-m', 'alt')
        fio.wait(lambda: ff.prompt_shown(rows(), 'M-x '), args.timeout, 'M-x prompt', process)
        keys(env, 'gnosis-dashboard', '-k', 'Return')
        fio.wait(lambda: any('Gnosis Dashboard' in row or 'Studied today' in row for row in rows()),
                 args.timeout, 'gnosis dashboard', process)
        try:
            fio.wait(lambda: any('Studied today' in row for row in rows())
                     and not any('Loading statistics' in row for row in rows()),
                     args.timeout, 'dashboard statistics', process)
        finally:
            time.sleep(2)
            capture('gnosis-dashboard')
        shown = rows()
        check('dashboard shows statistics, not the loading line',
              any('Studied today' in r for r in shown) and any('Review streak' in r for r in shown)
              and not any('Loading statistics' in r for r in shown), shown)
        # The command's idle period: its probe, then time for a second one
        # (a repeating idle timer runs once per idle period).
        try:
            fio.wait(lambda: idle_probes(after_dashboard(records(state)) or []), args.timeout, 'idle probe after the command', process)
            time.sleep(1.5)
        except AssertionError:
            pass
        # The keys' own idle periods: only the M-x prompt's waits for a key,
        # each at >= 1s of idleness and once per idle period.
        lines = records(state)
        done = after_dashboard(lines)
        between = idle_probes(lines[:len(lines) - len(done) if done is not None else len(lines)])[1:]
        periods = [commands for *_, commands in idle_probes(lines)]
        check('idle probes between the keys come only from the M-x prompt, once per idle period',
              done is not None and all(depth == 1 and idleness >= 1.0 for _, idleness, depth, _ in between)
              and len(set(periods)) == len(periods), lines)
        after = idle_probes(done or [])
        check('the command started a new idle period (one more idle probe)',
              len(after) == 1 and after[0][1] >= 1.0 and after[0][2] == 0, lines)
        report['records'] = records(state)


def measured(binary, args, output, report):
    """Idle CPU without scene dumps: the same init and keys, each window
    once the evaluator is quiet (startup's timers and blinking done)."""
    with session(binary, args, output, 'idle', report, False) as (env, state, process, _):
        fio.wait(lambda: '(init)' in records(state), args.timeout, 'init.el after --loadup (idle session)', process)
        quiet(process, args.settle, args.timeout)
        report['idle_cpu_started'] = idle_cpu(process.pid, args.cpu_window)
        keys(env, '-M', 'alt', 'x', '-m', 'alt')
        time.sleep(1)
        keys(env, 'gnosis-dashboard', '-k', 'Return')
        # The command's end starts a new idle period: the idle probe records it.
        fio.wait(lambda: idle_probes(after_dashboard(records(state)) or []), args.timeout,
                 'dashboard command (idle session)', process)
        quiet(process, args.settle, args.timeout)
        report['idle_cpu_dashboard'] = idle_cpu(process.pid, args.cpu_window)


def paint_lines(text):
    """[(kind, presentation)] of the log's `paint-submitted' and `paint' lines."""
    found = []
    for line in text.splitlines():
        if line.startswith('{') and '"diagnostic":"paint' in line:
            try:
                record = json.loads(line)
            except ValueError:
                continue
            if record.get('diagnostic') in ('paint-submitted', 'paint'):
                found.append((record['diagnostic'], record['presentation']))
    return found


def retired(binary, args, output, report):
    """A paint no input and no timer follows retires when it renders: the
    render wakes the evaluator's wait.  Polling for it, or waiting out the
    render stall (RETIRE_STALL seconds here), would leave it in flight."""
    def check(name, ok, detail):
        report['checks'].append({'check': name, 'ok': bool(ok)})
        if not ok:
            report['failures'].append({'check': name, 'detail': detail})
    with session(binary, args, output, 'retire', report, False, RETIRE_INIT,
                 {'ROMACS_RENDER_STALL_MS': str(RETIRE_STALL * 1000)}) as (_, _, process, log_path):
        text = lambda: log_path.read_text(errors='replace')
        fio.wait(lambda: 'retire probe' in text(), args.timeout, 'the retire probe message', process)
        # Every paint from the message on (showing it may take more than
        # one): once they stop coming, the last submitted must have rendered.
        def paints():
            return paint_lines(text().split('retire probe', 1)[1])
        def last(kind):
            return max((p for k, p in paints() if k == kind), default=0)
        fio.wait(lambda: last('paint-submitted'), 15, "the probe message's paint submitted", process)
        settled, count = time.monotonic(), len(paints())
        while time.monotonic() - settled < 2:
            time.sleep(.1)
            if len(paints()) != count:
                settled, count = time.monotonic(), len(paints())
        try:
            fio.wait(lambda: last('paint') >= last('paint-submitted'), RETIRE_STALL / 3, 'the last paint retired', process)
        except Exception:  # noqa: BLE001
            pass
        check('a paint with no input or timer after it retires when it renders, not at the stall',
              last('paint') >= last('paint-submitted'), {'tail': paints()[-6:]})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype and grim')
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--cpu-window', type=float, default=20)
    parser.add_argument('--settle', type=float, default=3, help='quiet evaluator seconds before an idle window')
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': [], 'failures': []}
    try:
        checked(binary, args, output, report)
        measured(binary, args, output, report)
        retired(binary, args, output, report)
        report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('idle_cpu_started', 'idle_cpu_dashboard', 'failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
