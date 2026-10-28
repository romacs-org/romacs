#!/usr/bin/env python3
"""Lisp timers in a native --loadup session, in a private headless compositor.

A private init.el installs probes that append to a state file: an ordinary
timer due 2s after init, a repeating 1s idle timer that records
`current-idle-time' and messages, and at 3s gnosis-feed's dwell probe (a timed
read that a timer's queued input must wake).  The session then sits idle (the idle probe
must fire once, at >= 1s of idleness, the ordinary one once), and its CPU time
over an idle window is read from /proc.  Real keys then run
M-x gnosis-dashboard, whose statistics arrive by `run-with-timer' and whose
link audit follows by `run-with-idle-timer': "Loading statistics..." must give
way to them.  Idle CPU is measured again once the dashboard has settled, and a
second key must start a new idle period (one more idle probe).

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY; DISPLAY is never set.  labwc, wlr-randr, wtype and grim come
from --tools (the gate's headless tools) or PATH.
"""
import argparse
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
(defun timers-live-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(timers-live-record '(init))
(run-with-timer 2 nil (lambda () (timers-live-record (list 'ordinary (and (current-idle-time) t)))))
(run-with-timer 3 nil (lambda () (require 'gnosis-feed) (timers-live-record (list 'dwell (gnosis-feed--dwell-supported-p)))))
(run-with-idle-timer 1 t
  (lambda ()
    (setq timers-live-idle (1+ timers-live-idle))
    (timers-live-record (list 'idle timers-live-idle (float-time (current-idle-time))))
    (message "Idle probe %d" timers-live-idle)))
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype and grim')
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--cpu-window', type=float, default=20)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': [], 'failures': []}
    compositor = keyboard = process = None
    try:
        with tempfile.TemporaryDirectory(prefix='timers-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config, work / 'gnosis'):
                directory.mkdir()
            state = work / 'timers-state.el'
            (home / 'config/romacs').mkdir(parents=True)
            (home / 'config/romacs/init.el').write_text(
                INIT.format(state=json.dumps(str(state)), gnosis=json.dumps(str(work / 'gnosis') + '/')))
            (config / 'rc.xml').write_text(live.RC_XML)
            path = os.environ.get('PATH', '/usr/bin:/bin')
            env = {'PATH': f'{args.tools}:{path}' if args.tools else path,
                   'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '14'}
            for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_GATE_FONTCONFIG'):
                if key in os.environ:
                    env[key] = os.environ[key]
            if 'ROMACS_GATE_FONTCONFIG' in env:
                env['FONTCONFIG_FILE'] = env['ROMACS_GATE_FONTCONFIG']
            log_path = output / 'binary.stderr'
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                socket = root / env['WAYLAND_DISPLAY']
                assert 'DISPLAY' not in env and socket.is_socket() and socket.parent == root
                report['display'] = {'wayland_socket': str(socket), 'compositor_pid': compositor.pid}
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)
                # A held key gives the seat a keyboard before the client starts.
                keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(binary), '--loadup', '--trace-scene'], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                child_env = Path(f'/proc/{process.pid}/environ').read_bytes().split(b'\0')
                report['display']['client_wayland_display'] = next(
                    (e.decode() for e in child_env if e.startswith(b'WAYLAND_DISPLAY=')), None)
                report['display']['client_has_display'] = any(e.startswith(b'DISPLAY=') for e in child_env)

                def rows():
                    return ff.current_rows(log_path.read_text(errors='replace'))

                def records():
                    return [line for line in state.read_text().splitlines()] if state.exists() else []

                def keys(*parts):
                    subprocess.run(['wtype', '-d', '40', *parts], env=env, check=True, timeout=20)
                    time.sleep(.15)

                def check(name, ok, detail):
                    report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
                    if not ok:
                        report['failures'].append(name)

                def capture(name):
                    report['captures'].append({'name': name, 'rows': rows()})
                    fio.shot(output / f'{name}.png', env, report)

                fio.wait(lambda: '(init)' in records(), args.timeout, 'init.el after --loadup', process)
                fio.wait(rows, args.timeout, 'first frame', process)
                # Idle probe: fires once at >= 1s of idleness; the ordinary timer once.
                # Not fio.wait: a binary without timers still gets its idle CPU measured.
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline and not (any(r.startswith('(idle 1 ') for r in records())
                                                           and any(r.startswith('(ordinary') for r in records())):
                    time.sleep(.1)
                time.sleep(.5)
                capture('idle-probe')
                first = records()
                idle_lines = [r for r in first if r.startswith('(idle')]
                check('idle probe fired once with >= 1s idleness', len(idle_lines) == 1
                      and float(idle_lines[0].split()[2].rstrip(')')) >= 1.0, first)
                check('ordinary timer fired once, while idle', first.count('(ordinary t)') == 1, first)
                fio.wait(lambda: any(r.startswith('(dwell') for r in records()), 15, 'dwell probe', process)
                check("a timer's queued input wakes a read (gnosis-feed--dwell-supported-p)",
                      '(dwell t)' in records(), records())
                check('echo area shows the idle probe', any('Idle probe 1' in row for row in rows()), rows()[-2:])
                report['idle_cpu_started'] = idle_cpu(process.pid, args.cpu_window)
                check('no more idle probes without input', len([r for r in records() if r.startswith('(idle')]) == 1,
                      records())

                keys('-M', 'alt', 'x', '-m', 'alt')
                fio.wait(lambda: rows()[-1:] == ['M-x '] or rows()[:1] == ['M-x '], args.timeout, 'M-x prompt', process)
                keys('gnosis-dashboard', '-k', 'Return')
                fio.wait(lambda: any('Gnosis Dashboard' in row or 'Studied today' in row for row in rows()),
                         args.timeout, 'gnosis dashboard', process)
                try:
                    fio.wait(lambda: any('Studied today' in row for row in rows())
                             and not any('Loading statistics' in row for row in rows()),
                             15, 'dashboard statistics', process)
                finally:
                    time.sleep(2)
                    capture('gnosis-dashboard')
                shown = rows()
                check('dashboard shows statistics, not the loading line',
                      any('Studied today' in r for r in shown) and any('Review streak' in r for r in shown)
                      and not any('Loading statistics' in r for r in shown), shown)
                after_command = [r for r in records() if r.startswith('(idle')]
                check('the command started a new idle period (one more idle probe)', len(after_command) == 2, records())
                report['idle_cpu_dashboard'] = idle_cpu(process.pid, args.cpu_window)
                report['records'] = records()
                report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        live.stop(process)
        live.stop(keyboard)
        live.stop(compositor)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('idle_cpu_started', 'idle_cpu_dashboard', 'failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
