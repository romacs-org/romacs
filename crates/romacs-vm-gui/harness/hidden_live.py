#!/usr/bin/env python3
"""A window on a hidden workspace, in a native --loadup session in a private
headless labwc with two desktops.

Sessions died a little after the user switched Hyprland workspaces: a
window the compositor does not draw gets no frame callbacks, winit then
withholds RedrawRequested, the paint in flight never renders, and the loop
gave up after 30 s.  Here labwc's GoToDesktop (bound to a key wtype sends)
hides the window the same way.  A private init.el runs a repeating timer that
inserts a line into the shown buffer, so a paint is in flight while hidden,
and records each tick in a state file.

Checks: while hidden no paint renders (the scenario holds) and the session
outlives ROMACS_RENDER_STALL_MS several times over; the timer keeps running;
the stall is told in exactly one stderr line; the evaluator does not spin.
Shown again, the window repaints, keeps repainting, and typed keys reach the
buffer.  Every process gets a private HOME/XDG tree and the compositor's
WAYLAND_DISPLAY; DISPLAY is never set.  labwc, wlr-randr, wtype and grim come
from --tools or PATH.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import fileio_live as fio
import live_oracle as live
import timers_live as tl

RC_XML = '''<?xml version="1.0"?>
<labwc_config>
  <core><gap>0</gap></core>
  <desktops number="2"/>
  <keyboard>
    <keybind key="W-F1"><action name="GoToDesktop" to="1"/></keybind>
    <keybind key="W-F2"><action name="GoToDesktop" to="2"/></keybind>
  </keyboard>
  <windowRules><windowRule identifier="*" serverDecoration="no">
    <action name="Maximize"/>
  </windowRule></windowRules>
</labwc_config>
'''

INIT = '''(setq inhibit-startup-screen t)
(defvar hidden-live-ticks 0)
(defun hidden-live-record (form)
  (write-region (concat (prin1-to-string form) "\\n") nil {state} 'append 'silent))
(hidden-live-record '(init))
(run-with-timer 0.5 0.5
  (lambda ()
    (setq hidden-live-ticks (1+ hidden-live-ticks))
    (with-current-buffer "*scratch*"
      (save-excursion (goto-char (point-min)) (insert (format "tick %d\\n" hidden-live-ticks)))
      (hidden-live-record
       (list 'tick hidden-live-ticks (float-time)
             (delq nil (mapcar (lambda (typed) (and (string-match-p typed (buffer-string)) typed)) '("zz" "yy"))))))))
'''


def paints(text):
    """Presentations the renderer drew, in order (`paint' diagnostics)."""
    return [json.loads(line)['presentation'] for line in text.splitlines()
            if line.startswith('{') and '"diagnostic":"paint"' in line]


def ticks(state):
    """Each tick's (N, its record), from (tick N TIME TYPED)."""
    return [(int(line.split(' ', 2)[1]), line) for line in tl.records(state) if line.startswith('(tick ')]


def typed(state, text):
    return any(f'"{text}"' in line for _, line in ticks(state))


def chord(env, modifier, key):
    subprocess.run(['wtype', '-M', modifier, '-k', key, '-m', modifier], env=env, check=True, timeout=20)
    time.sleep(.15)


def drawn_prompts(text):
    """Prompt paints the renderer drew: those drawn at once, and those a
    read later saw drawn (prompt.rs `settle_drawn'); the prompt never waits
    for drawing."""
    return text.count('"diagnostic":"prompt-paint","drawn":true') + text.count('"diagnostic":"prompt-drawn"')


def grab(env, path):
    """The output's pixels as PNG bytes."""
    subprocess.run(['grim', str(path)], env=env, check=True, timeout=10)
    return path.read_bytes()


def stalls(text):
    return [line for line in text.splitlines() if line.startswith('romacs: render stalled:')]


def hidden_round(name, args, env, state, process, text, report, check, warnings):
    """Hide the window for --hidden seconds; whether the session lived."""
    chord(env, 'logo', 'F2')
    time.sleep(1)  # a paint drawn as the switch lands
    hidden_at, drawn_at_hide, ticks_at_hide = time.monotonic(), len(paints(text())), len(ticks(state))
    # The loop polls every millisecond until the paint counts as stalled:
    # measure the steady state after that.
    settle = args.stall + 0.5
    time.sleep(settle)
    cpu = tl.idle_cpu(process.pid, max(args.hidden - 1 - settle, 2))
    hidden_for = time.monotonic() - hidden_at + 1
    alive = process.poll() is None
    drawn_hidden = paints(text())[drawn_at_hide:]
    ticks_hidden = len(ticks(state)) - ticks_at_hide
    told = stalls(text())
    report.setdefault('hidden', {})[name] = {'seconds': round(hidden_for, 1), 'drawn': drawn_hidden,
                                             'ticks': ticks_hidden, 'warnings': told, 'cpu': cpu}
    check(f'{name} hidden: no paint renders (frame callbacks withheld)', not drawn_hidden, drawn_hidden)
    check(f'{name} hidden: session alive after {hidden_for:.0f}s, stall {args.stall}s', alive,
          {'returncode': process.poll(), 'tail': text().splitlines()[-3:]})
    check(f'{name} hidden: the timer keeps running', ticks_hidden >= int(hidden_for / 0.5) - 3,
          {'ticks': ticks_hidden, 'seconds': hidden_for})
    check(f'{name} hidden: at most one stall warning', len(told) in warnings, told)
    evaluator = next((v for k, v in cpu['threads'].items() if k.startswith('romacs-evaluat')), None)
    check(f'{name} hidden: the evaluator does not spin',
          evaluator is None or evaluator['wakeups_per_second'] < 200, evaluator)
    return alive


def shown_round(name, env, state, process, text, report, check, keys, before_keys, drawn):
    """Show the window again: it repaints (DRAWN of the log counts the
    paints drawn), and KEYS typed reach the buffer."""
    chord(env, 'logo', 'F1')
    shown_at, drawn_at_show = time.monotonic(), drawn(text())
    warnings = len(stalls(text()))
    try:
        fio.wait(lambda: drawn(text()) >= drawn_at_show + 2, 10, 'repaints once shown', process)
        report.setdefault('repaint_seconds', {})[name] = round(time.monotonic() - shown_at, 2)
        check(f'{name} shown: the window repaints and keeps repainting', True, report['repaint_seconds'][name])
    except AssertionError as error:
        check(f'{name} shown: the window repaints and keeps repainting', False, str(error))
    before_keys()
    tl.keys(env, keys)
    try:
        fio.wait(lambda: typed(state, keys), 10, 'typed keys in the buffer', process)
        check(f'{name} shown: typed keys reach the buffer', True, keys)
    except AssertionError as error:
        check(f'{name} shown: typed keys reach the buffer', False, str(error))
    check(f'{name} shown: no stall warning once drawn', len(stalls(text())) == warnings, stalls(text()))


def run(args, output, report):
    def check(name, ok, detail):
        report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
        if not ok:
            report['failures'].append(name)

    compositor = keyboard = process = None
    try:
        with tempfile.TemporaryDirectory(prefix='hidden-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            env, state, work, config = tl.session_env(root, args)
            (Path(env['HOME']) / 'config/romacs/init.el').write_text(INIT.format(state=json.dumps(str(state))))
            (config / 'rc.xml').write_text(RC_XML)
            env.update({'ROMACS_DIAGNOSTICS': '1', 'ROMACS_RENDER_STALL_MS': str(int(args.stall * 1000))})
            if args.present_mode:
                env['ROMACS_BENCH_PRESENT_MODE'] = args.present_mode
            log_path = output / 'session.stderr'
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                assert 'DISPLAY' not in env
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)
                # A held key gives the seat a keyboard before the client starts.
                keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                empty = grab(env, output / 'empty.png')
                process = subprocess.Popen([str(args.binary), '--loadup'], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                report['pid'] = process.pid
                text = lambda: log_path.read_text(errors='replace')
                if args.hidden_start:
                    # Hidden as it starts (the owner switches away while it
                    # prepares its world): once mapped, by the first start's
                    # "Preparing" frame or the first paint, its next frame is
                    # not drawn until it shows.
                    # Mapped once the output differs from the empty desktop
                    # (a window maps on the desktop current when it does).
                    fio.wait(lambda: grab(env, output / 'start.png') != empty, args.timeout, 'the window drawn', process)
                    first_paint = 'paint-submitted' in text()
                    chord(env, 'logo', 'F2')
                    fio.wait(lambda: len(ticks(state)) >= 2 * args.stall + 4, args.timeout, 'ticks while hidden from the start', process)
                    report['hidden_start'] = {'drawn': paints(text()), 'warnings': stalls(text()),
                                              'first_paint_before_hiding': first_paint}
                    check('hidden from the start: hidden before the first paint', not first_paint, report['hidden_start'])
                    check('hidden from the start: alive, no paint drawn', process.poll() is None and not paints(text()),
                          report['hidden_start'])
                    chord(env, 'logo', 'F1')
                fio.wait(lambda: len(ticks(state)) >= 2 and paints(text()), args.timeout, 'ticks and a drawn paint', process)
                fio.shot(output / 'before.png', env, report)

                told = len(stalls(text()))
                if not hidden_round('plain', args, env, state, process, text, report, check, (told + 1,)):
                    return
                shown_round('plain', env, state, process, text, report, check, 'zz', lambda: None,
                            lambda log: len(paints(log)))
                fio.shot(output / 'after.png', env, report)
                # Again with a minibuffer prompt open: the timer's redisplays
                # go through the prompt's host (prompt.rs), which must not
                # block on, or fail over, the undrawn paint either.
                prompts = lambda: text().count('"diagnostic":"prompt-paint"')
                chord(env, 'alt', 'x')
                fio.wait(lambda: prompts() > 0, 10, 'the M-x prompt drawn', process)
                # The loop that tells a stall waits for the prompt to end.
                if not hidden_round('prompt', args, env, state, process, text, report, check, (told + 1, told + 2)):
                    return
                shown_round('prompt', env, state, process, text, report, check, 'yy', lambda: chord(env, 'ctrl', 'g'),
                            drawn_prompts)
                fio.shot(output / 'after-prompt.png', env, report)
    finally:
        live.stop(process)
        live.stop(keyboard)
        live.stop(compositor)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype and grim')
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--stall', type=float, default=2, help='ROMACS_RENDER_STALL_MS, in seconds')
    parser.add_argument('--hidden', type=float, default=10, help='seconds on the other desktop')
    parser.add_argument('--hidden-start', action='store_true', help='hide the window as the session starts, too')
    parser.add_argument('--present-mode', choices=('fifo', 'mailbox'), help='the surface present mode (default: mailbox if supported)')
    args = parser.parse_args()
    args.binary = Path(args.binary).resolve()
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'binary': str(args.binary), 'stall': args.stall, 'status': 'FAIL', 'checks': [], 'failures': []}
    try:
        run(args, output, report)
        report['status'] = 'PASS' if report['checks'] and not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('hidden', 'repaint_seconds', 'failures', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
