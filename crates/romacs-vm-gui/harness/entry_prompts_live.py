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
from pathlib import Path
import subprocess
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
    state = None

    def prepare(session):
        nonlocal state
        state = session.work / 'state.el'
        (session.home / 'config/romacs/init.el').write_text(INIT.format(state=json.dumps(str(state))))

    with live.session(args.binary, output, prefix='entry-prompts-', tools=args.tools, prepare=prepare, report=report) as session:
        process, env = session.process, session.env

        def rows():
            return ff.current_rows(session.log.read_text(errors='replace'))

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
    print(report['status'], json.dumps({k: report.get(k) for k in ('cases', 'error')}, ensure_ascii=False))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
