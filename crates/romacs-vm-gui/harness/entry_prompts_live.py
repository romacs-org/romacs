#!/usr/bin/env python3
"""Prompts opened outside a command leave the screen once answered.

A native --loadup session in a private headless compositor, with a private
init.el binding two keys: F5 arms a timer and F6 starts a background command,
whose callbacks each read a string.  The prompt is answered with RET, which
changes nothing shown, so only the prompt itself makes the frame differ from
the last paint before it.  Each prompt must appear, and after RET the
submitted scene must no longer show it.  No other timers run: nothing else
repaints to hide a prompt left on screen.

usage: entry_prompts_live.py --binary PATH --output DIR [--tools DIR:DIR]
"""
import argparse
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

INIT = '''(defun entry-prompts-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(global-set-key [f5] (lambda () (interactive)
  (run-with-timer 0.3 nil (lambda () (entry-prompts-record (list 'timer (read-string "Timer prompt: ")))))))
(global-set-key [f6] (lambda () (interactive)
  (romacs-process-start (list "true") "" default-directory 10
    (lambda (_) (entry-prompts-record (list 'background (read-string "Background prompt: "))) nil))))
(entry-prompts-record '(init))
'''
CASES = (('timer', 'F5', 'Timer prompt:'), ('background', 'F6', 'Background prompt:'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directories with labwc, wlr-randr and wtype, PATH-style')
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    output = Path(args.output).resolve()
    output.mkdir(parents=True)
    report = {'binary': str(Path(args.binary).resolve()), 'status': 'FAIL', 'cases': {}}
    compositor = keyboard = process = None
    temporary = None
    try:
        with tempfile.TemporaryDirectory(prefix='entry-prompts-', dir=os.environ.get('XDG_RUNTIME_DIR'), delete=False) as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config, home / 'config/romacs'):
                directory.mkdir(parents=True)
            state = work / 'state.el'
            (home / 'config/romacs/init.el').write_text(INIT.format(state=json.dumps(str(state))))
            (config / 'rc.xml').write_text(live.RC_XML)
            path = os.environ.get('PATH', '/usr/bin:/bin')
            env = {'PATH': f'{args.tools}:{path}' if args.tools else path, 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
            for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_WORLD'):
                if key in os.environ:
                    env[key] = os.environ[key]
            log_path = output / 'binary.stderr'
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or 'labwc', '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([shutil.which('wlr-randr', path=env['PATH']) or 'wlr-randr', '--output', 'HEADLESS-1',
                                '--custom-mode', '1280x800'], env=env, check=True, timeout=15)
                keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(Path(args.binary).resolve()), '--loadup', '--trace-scene'], env=env,
                                           cwd=work, stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def rows():
                    return ff.current_rows(log_path.read_text(errors='replace'))

                def shown(text):
                    return any(text in row for row in rows())

                def records():
                    return state.read_text().splitlines() if state.exists() else []

                fio.wait(lambda: '(init)' in records(), args.timeout, 'init.el after --loadup', process)
                fio.wait(rows, args.timeout, 'first frame', process)
                time.sleep(2)
                for name, key, prompt in CASES:
                    subprocess.run(['wtype', '-k', key], env=env, check=True, timeout=20)
                    fio.wait(lambda: shown(prompt), args.timeout, f'{name} prompt shown', process)
                    subprocess.run(['wtype', '-k', 'Return'], env=env, check=True, timeout=20)
                    fio.wait(lambda: f'({name} "")' in records(), args.timeout, f'{name} prompt answered', process)
                    # A loaded host may repaint seconds late; a prompt that is
                    # never taken down stays past what a hang takes.
                    deadline = time.monotonic() + args.timeout
                    while shown(prompt) and time.monotonic() < deadline:
                        time.sleep(.1)
                    report['cases'][name] = {'answered': True, 'prompt_left_screen': not shown(prompt),
                                             'rows': rows()[-2:]}
                report['status'] = 'PASS' if all(c['prompt_left_screen'] for c in report['cases'].values()) else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        for proc in (process, keyboard, compositor):
            live.stop(proc)
        try:
            (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
        finally:
            live.remove_root(temporary)
    print(report['status'], json.dumps({k: report.get(k) for k in ('cases', 'error')}, ensure_ascii=False))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
