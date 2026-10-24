#!/usr/bin/env python3
"""Owner-path probe: GNU-only GUI, real private wl_keyboard input, no eval RPC.
Run from repository root with the exact built binary. Uses the existing GUI
harness's private labwc setup; no source/Lisp modifications or stand-ins.
Screenshots and raw submitted-scene diagnostics are evidence, not self-tests.
"""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'crates/romacs-vm-gui/harness'))
import live_oracle as live


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--startup-timeout', type=float, default=60)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=False)
    binary = a.binary.resolve()
    report = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'launch': 'GNU-only; no stand-in commands installed', 'steps': [],
              'input': 'wtype -> private labwc virtual keyboard -> native wl_keyboard',
              'compositor': live.LABWC, 'renderer': 'pixman'}
    comp = keyboard = proc = None
    with tempfile.TemporaryDirectory(prefix='mxg-', dir=str(_PortablePath(_portable_tempfile.gettempdir()) / '')) as temp:
        runtime = Path(temp)
        runtime.chmod(0o700)
        home = runtime / 'home'; home.mkdir()
        config = runtime / 'config'; config.mkdir()
        (config / 'rc.xml').write_text(live.RC_XML)
        fixture = runtime / 'owner-file.txt'
        fixture.write_text('OWNER FILE JOURNEY 3c1166fc\nReal file, opened by C-x C-f.\n')
        report['fixture_path'] = str(fixture)
        report['fixture_text'] = fixture.read_text()
        env = {'PATH': os.environ['PATH'], 'HOME': str(home), 'LANG': 'C.UTF-8',
               'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(runtime),
               'WLR_BACKENDS': 'headless', 'WLR_HEADLESS_OUTPUTS': '1', 'WLR_RENDERER': 'pixman',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
        with (a.output / 'compositor.log').open('w') as clog, (a.output / 'editor.stderr').open('w') as elog, (a.output / 'editor.stdout').open('w') as olog:
            try:
                comp = subprocess.Popen([live.LABWC, '-C', str(config)], env=env, stdout=clog, stderr=clog)
                deadline = time.monotonic() + 20
                while not any(s.is_socket() for s in runtime.glob('wayland-*')):
                    if comp.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError('private compositor startup failed')
                    time.sleep(.1)
                env['WAYLAND_DISPLAY'] = next(s for s in runtime.glob('wayland-*') if s.is_socket()).name
                subprocess.run([live.RANDR, '--output', 'HEADLESS-1', '--custom-mode', '1000x700'], env=env, check=True, timeout=10)
                keyboard = subprocess.Popen(['/usr/bin/wtype', '-s', '120000', '-k', 'Shift_L'], env=env, stdout=elog, stderr=elog)
                time.sleep(.3)
                cmd = [str(binary), '--gnu-lisp', '--geometry', '1000x700', '--trace-scene']
                report['command'] = cmd
                proc = subprocess.Popen(cmd, cwd=ROOT, env=env, stdin=subprocess.DEVNULL, stdout=olog, stderr=elog)
                report['startup_budget_seconds'] = a.startup_timeout
                deadline = time.monotonic() + a.startup_timeout
                while time.monotonic() < deadline and proc.poll() is None:
                    olog.flush()
                    if '"ready":true' in (a.output / 'editor.stdout').read_text():
                        break
                    time.sleep(.1)
                report['startup_exit'] = proc.poll()
                report['startup_ready'] = '"ready":true' in (a.output / 'editor.stdout').read_text()

                def step(name, sequences):
                    row = {'step': name, 'keys': sequences, 'process_exit_before': proc.poll()}
                    if proc.poll() is None and report['startup_ready']:
                        before = (a.output / 'editor.stderr').stat().st_size
                        for seq in sequences:
                            subprocess.run(['/usr/bin/wtype', '-d', '100', *seq], env=env, check=True, timeout=20)
                            time.sleep(.3)
                        time.sleep(1)
                        row['input_delivered'] = True
                        row['process_exit_after'] = proc.poll()
                        row['stderr_delta'] = (a.output / 'editor.stderr').read_text()[before:]
                    else:
                        row['input_delivered'] = False
                        row['blocked_reason'] = 'GUI did not reach ready; later input not executed'
                    png = a.output / (name + '.png')
                    subprocess.run(['grim', str(png)], env=env, check=True, timeout=10)
                    row['screenshot'] = png.name
                    report['steps'].append(row)

                step('00-startup', [])
                step('01-mx-prompt', [['-M', 'alt', '-k', 'x', '-m', 'alt']])
                step('02-mx-complete', [['forward-ch'], ['-k', 'Tab']])
                step('03-mx-run', [['-k', 'Return']])
                step('04-cancel', [['-M', 'ctrl', '-k', 'g', '-m', 'ctrl']])
                step('05-find-file-prompt', [['-M', 'ctrl', '-k', 'x', '-m', 'ctrl'], ['-M', 'ctrl', '-k', 'f', '-m', 'ctrl']])
                step('06-find-file-run', [[str(fixture)], ['-k', 'Return']])
                report['process_exit_before_cleanup'] = proc.poll()
            except Exception as e:
                report['probe_error'] = repr(e)
            finally:
                for child in (proc, keyboard, comp):
                    live.stop(child)
                report['cleanup_complete'] = all(x is None or x.poll() is not None for x in (proc, keyboard, comp))
        (a.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'steps'}, indent=2))
    return 1 if report.get('probe_error') else 0


if __name__ == '__main__':
    raise SystemExit(main())
