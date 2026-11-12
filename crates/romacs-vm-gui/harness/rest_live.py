#!/usr/bin/env python3
"""The render thread at rest, in a native --loadup session in a private
headless labwc: what it draws, and why, while nothing on screen changes.

The session runs under ROMACS_DIAGNOSTICS=1, so its render thread writes
its frame counters as `frame-stats' lines (frame_stats.rs): frames planned
and presented per demand reason, redraw requests and events, scene commits
and loop wakeups, a line a second while they move and none once they rest.
Real keys (wtype) run the init's commands, which put the session in each
state; a state's frames are the last line after it less the last before.

  idle       *scratch*, `blink-cursor-mode' off, nothing running: no frame
             is presented and the main thread uses under 1% of a core.
  blink      GNU's default blink after a command: the blink's own frames
             (two a second while it blinks) and no others.
  chat       an agent chat beside *scratch*, its turn running: the live
             rule's clock, one frame a second, and no others.
  chat-poll  the same with a timer 20 times a second that changes nothing
             (the rate of the MCP endpoint's deferred poll): still the
             clock's frames alone.

Every frame presented in any state answers a scene the evaluator sent
(`scene_commits') and names `redisplay' alone as its reason.  A frame the
render thread asked for itself (an effect left standing, a platform
redraw) fails the state it shows up in.

The chat is a session made in place, with no harness behind it: its state
is `running', which is all the live rule and its timer read.

Sourcing romacs-gpu-env.sh first runs the private compositor on the real
GPU, where effects the software renderer suppresses can stand.  Every
process gets a private HOME/XDG tree and the compositor's WAYLAND_DISPLAY;
DISPLAY is never set.  labwc, wlr-randr and wtype come from --tools or PATH.

usage: rest_live.py --binary PATH --output DIR [--seconds 6] [--tools DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time

import fileio_live as fio
import timers_live as tl

# The session starts in its work directory, where its records go.
INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil auto-save-timeout nil)
(blink-cursor-mode -1)
(defvar rest-live-work command-line-default-directory)
(defun rest-live-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil (expand-file-name "rest-state.el" rest-live-work)
                'append 'silent))
(defmacro rest-live-state (key name &rest body)
  `(global-set-key (kbd ,key) (lambda () (interactive) ,@body (rest-live-record '(,name)))))
(rest-live-state "<f5>" blink (blink-cursor-mode 1))
(rest-live-state "<f6>" chat
  (blink-cursor-mode -1)
  (require 'romacs-agent)
  (let ((session (romacs-agent--make :id "0a0a0a0a-0000-4000-8000-000000000001" :harness 'claude
                                     :provider "claude" :title "at rest" :state 'running
                                     :turn-start (float-time) :dir rest-live-work :cwd rest-live-work)))
    (puthash (romacs-agent-id session) session romacs-agent--sessions)
    (delete-other-windows)
    (select-window (split-window-right))
    (switch-to-buffer (romacs-agent-show session))))
(rest-live-state "<f7>" chat-poll (run-at-time 0.05 0.05 #'ignore))
(rest-live-record '(init))
'''

COUNTS = ('wakeups', 'redraw_requests', 'redraw_events', 'scene_commits', 'presents')


def stats(text):
    """The `frame-stats' lines of TEXT, in order."""
    rows = []
    for line in text.splitlines():
        if line.startswith('{') and '"diagnostic":"frame-stats"' in line:
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return rows


def frames(first, last):
    """LAST's counters less FIRST's: the totals, the frames presented per
    reason, and the seconds between the two lines."""
    out = {name: last[name] - first[name] for name in COUNTS}
    reasons = {name: count - first['presented'].get(name, 0) for name, count in last['presented'].items()}
    out['presented'] = {name: count for name, count in reasons.items() if count}
    out['seconds'] = round((last['at_us'] - first['at_us']) / 1e6, 2)
    return out


def main_thread_percent(pid, seconds):
    """Percent of one core PID's main thread used over SECONDS."""
    stat = Path(f'/proc/{pid}/task/{pid}/stat')
    ticks = lambda: sum(int(f) for f in stat.read_text().rsplit(')', 1)[1].split()[11:13])
    before, start = ticks(), time.monotonic()
    time.sleep(seconds)
    return round(100 * (ticks() - before) / os.sysconf('SC_CLK_TCK') / (time.monotonic() - start), 2)


def held(log, pid, seconds):
    """The session's frames over SECONDS of its present state, and the main
    thread's share of a core.  The counters' last line is at most a second
    behind them, so it is read a moment after each end of the window."""
    time.sleep(1.5)
    first = stats(log.read_text(errors='replace'))[-1]
    percent = main_thread_percent(pid, seconds)
    time.sleep(1.5)
    return {**frames(first, stats(log.read_text(errors='replace'))[-1]), 'window_seconds': seconds,
            'main_thread_percent': percent}


def measured(binary, args, output, report):
    def check(name, ok, detail):
        report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
        if not ok:
            report['failures'].append(name)

    def evaluators_only(name, state):
        """Each frame answers a scene the evaluator sent.  The reason alone
        does not say so: a frame the render pass asked to follow (an effect
        it is still drawing) is named `redisplay' too, beside its effect."""
        others = {reason: count for reason, count in state['presented'].items() if reason != 'redisplay'}
        check(f'{name}: no frame names a reason but redisplay', not others, state['presented'])
        check(f'{name}: no frame but those the evaluator sent', state['presents'] <= state['scene_commits'] + 1, state)

    # The real GPU, when asked for: the compositor stays private.
    gpu = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
    with tl.session(binary, args, output, 'rest', report, False, init=INIT, extra_env=gpu) as (env, state, process, log):
        records = state.with_name('rest-state.el')
        seconds = args.seconds

        def enter(key, name):
            tl.keys(env, '-k', key)
            fio.wait(lambda: f'({name})' in tl.records(records), 30, f'state {name}', process)

        fio.wait(lambda: '(init)' in tl.records(records), args.timeout, 'init', process)
        fio.wait(lambda: stats(log.read_text(errors='replace')), args.timeout, 'the first frame-stats line', process)
        tl.quiet(process, 2, args.timeout)
        report['renderer'] = next((line for line in log.read_text(errors='replace').splitlines()
                                   if line.startswith('romacs: gpu ')), None)

        idle = report['idle'] = held(log, process.pid, seconds)
        check('idle: no frame is presented', idle['presents'] == 0 and not idle['presented'], idle)
        check('idle: no redraw is asked for', idle['redraw_requests'] == 0 and idle['redraw_events'] == 0, idle)
        check('idle: the main thread uses under 1% of a core', idle['main_thread_percent'] < 1, idle)

        # The blink starts half a second into the idle period the key's
        # command ends with, then turns the cursor off or on each half second.
        enter('F5', 'blink')
        blink = report['blink'] = held(log, process.pid, seconds)
        evaluators_only('blink', blink)
        check('blink: it blinks', blink['presents'] >= blink['seconds'], blink)
        check('blink: two frames a second and no more', blink['presents'] <= 2 * blink['seconds'] + 2, blink)

        enter('F6', 'chat')
        tl.quiet(process, .5, args.timeout)
        chat = report['chat'] = held(log, process.pid, seconds)
        evaluators_only('chat', chat)
        check('chat: its clock ticks', chat['presents'] >= chat['seconds'] - 2, chat)
        check('chat: one frame a second and no more', chat['presents'] <= chat['seconds'] + 2, chat)

        enter('F7', 'chat-poll')
        poll = report['chat_poll'] = held(log, process.pid, seconds)
        evaluators_only('chat-poll', poll)
        check('chat-poll: a timer that changes nothing presents nothing', poll['presents'] <= poll['seconds'] + 2, poll)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--seconds', type=float, default=6, help='how long each state is held')
    parser.add_argument('--timeout', type=float, default=240)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr and wtype')
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'checks': [], 'failures': []}
    try:
        measured(binary, args, output, report)
    except (AssertionError, RuntimeError, OSError, IndexError) as error:
        report['error'] = repr(error)
    report['status'] = 'PASS' if report['checks'] and not report['failures'] and 'error' not in report else 'FAIL'
    (output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(report['status'], json.dumps({key: report.get(key) for key in
                                        ('idle', 'blink', 'chat', 'chat_poll', 'failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
