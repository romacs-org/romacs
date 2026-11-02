#!/usr/bin/env python3
"""`auto-revert-mode' through file notification, in a native --loadup
session in a private headless compositor.

A private init.el visits a file with `auto-revert-mode' on,
`auto-revert-use-notify' t and `auto-revert-avoid-polling' t, and a polling
interval far longer than the run, so only a file notification can revert
the buffer.  It records the library filenotify.el found and the buffer's
watch descriptor.  The harness then rewrites the file from outside, twice,
while the session sits idle: each time `after-revert-hook' must record the
new text, and a frame submitted after the write must show it.  The session
is then quiet: its idle CPU over a window shows the watch costs nothing
while nothing changes.

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY; DISPLAY is never set.  labwc, wlr-randr, wtype and grim come
from --tools (the gate's headless tools) or PATH.
"""
import argparse
import hashlib
import json
from pathlib import Path
import time

import fileio_live as fio
import find_file_live as ff
import timers_live as tl

INIT = '''(setq inhibit-startup-screen t)
(setq auto-revert-use-notify t auto-revert-avoid-polling t
      auto-revert-interval 300 auto-revert-verbose nil)
(defun autorevert-live-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(add-hook 'after-revert-hook
          (lambda () (autorevert-live-record
                       (list 'reverted (float-time) (replace-regexp-in-string "\n" "|" (buffer-string))))))
(defvar autorevert-live-file (expand-file-name "watched.txt" {gnosis}))
(write-region "initial\n" nil autorevert-live-file nil 'silent)
(find-file autorevert-live-file)
(auto-revert-mode 1)
(autorevert-live-record (list 'init (and (boundp 'file-notify--library) file-notify--library)
                              (and auto-revert-notify-watch-descriptor t)
                              auto-revert-use-notify))
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype and grim')
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--cpu-window', type=float, default=10)
    parser.add_argument('--settle', type=float, default=3)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': [], 'failures': []}

    def check(name, ok, detail):
        report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
        if not ok:
            report['failures'].append(name)

    # timers_live's session installs its INIT; this one's replaces it.
    tl.INIT = INIT
    try:
        with tl.session(binary, args, output, 'autorevert', report, True) as (env, state, process, log_path):
            work = Path(env['HOME']).parent / 'work'
            watched = work / 'gnosis' / 'watched.txt'
            records = lambda: tl.records(state)

            fio.wait(lambda: any(r.startswith('(init') for r in records()), args.timeout, 'init.el after --loadup', process)
            fio.wait(lambda: ff.current_rows(log_path.read_text(errors='replace')), args.timeout, 'first frame', process)
            init = next(r for r in records() if r.startswith('(init'))
            check('filenotify found inotify and the buffer has a watch', init == '(init inotify t t)', init)
            tl.quiet(process, args.settle, args.timeout)
            for n, text in enumerate(['changed outside once', 'changed outside twice'], 1):
                before = len([r for r in records() if r.startswith('(reverted')])
                written = time.time()
                watched.write_text(text + '\n')
                try:
                    fio.wait(lambda: len([r for r in records() if r.startswith('(reverted')]) > before
                             and any(text in row for row in ff.current_rows(log_path.read_text(errors='replace'))),
                             10, f'revert {n}', process)
                except Exception:  # noqa: BLE001
                    pass
                reverts = [r for r in records() if r.startswith('(reverted')]
                latest = reverts[-1] if len(reverts) > before else None
                latency = round(float(latest.split()[1]) - written, 3) if latest else None
                rows = ff.current_rows(log_path.read_text(errors='replace'))
                report.setdefault('reverts', []).append({'text': text, 'record': latest, 'latency_s': latency})
                check(f'external write {n} reverted the buffer by notification', latest is not None
                      and json.dumps(text + '|') in latest and latency is not None and latency < 5, latest)
                check(f'a frame shows external write {n}', any(text in row for row in rows), rows)
                report['captures'].append({'name': f'revert-{n}', 'rows': rows})
                fio.shot(output / f'revert-{n}.png', env, report)
            tl.quiet(process, args.settle, args.timeout)
            report['idle_cpu'] = tl.idle_cpu(process.pid, args.cpu_window)
            check('idle with a live watch stays under 1% of a core', report['idle_cpu']['percent_of_core'] < 1,
                  report['idle_cpu'])
            report['records'] = records()
        report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('reverts', 'idle_cpu', 'failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
