#!/usr/bin/env python3
"""`omarchy-mode' following a fake Omarchy in a live session.

One fresh private headless labwc and one `romacs-vm-gui -Q --loadup' (as
theme_live.py starts them), with the state directory of a fake Omarchy in
its private tree.  Each step writes what `omarchy-theme-set' writes, in its
order (the theme directory swapped, then theme.generation, then theme.name),
and the session's own `enable-theme-functions' log says when the theme
changed: real inotify, the real delay, the real command loop.

The run fails when a step's theme is not the one enabled, or takes more than
--deadline seconds from the last write, or the session does not answer an
M-: while it follows, or a theme by hand does not pause following until the
next change.

usage: omarchy_live.py --binary PATH --output DIR [--deadline 1] [--tools DIR:DIR]
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import os
import time

import perkey_live as perkey
import theme_live

PALETTE = '''mode = "dark"
accent = "#ff7a18"
background = "#101820"
foreground = "#e8e0d0"
red = "#e06c75"
green = "#98c379"
yellow = "#e5c07b"
blue = "#61afef"
magenta = "#c678dd"
cyan = "#56b6c2"
'''


def switch(state, generation, name, palette):
    """What `omarchy-theme-set' writes, in its order; when it ended."""
    staged = state / 'next-theme'
    staged.mkdir()
    if palette:
        (staged / 'colors.toml').write_text(palette)
    shutil.rmtree(state / 'theme', ignore_errors=True)
    staged.rename(state / 'theme')
    (state / 'theme.generation').write_text(f'{generation}\n')
    (state / 'theme.name').write_text(f'{name}\n')
    return time.time()


def staged_switch(state, generation, name, palette, gap):
    """`switch' with the writer stalled GAP seconds before the name."""
    (state / 'next-theme').mkdir()
    (state / 'next-theme' / 'colors.toml').write_text(palette)
    shutil.rmtree(state / 'theme', ignore_errors=True)
    (state / 'next-theme').rename(state / 'theme')
    (state / 'theme.generation').write_text(f'{generation}\n')
    time.sleep(gap)
    (state / 'theme.name').write_text(f'{name}\n')
    return time.time()


def enabled_after(log, seen, deadline):
    """The (THEME . TIME) rows `log' gains within DEADLINE seconds."""
    end = time.monotonic() + deadline
    while time.monotonic() < end:
        rows = [line.split() for line in log.read_text().splitlines()] if log.exists() else []
        if len(rows) > seen:
            time.sleep(.3)
            rows = [line.split() for line in log.read_text().splitlines()]
            return [row for row in rows[seen:] if row[0] != 'user']
        time.sleep(.01)
    return []


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--deadline', type=float, default=1)
    ap.add_argument('--settle', type=float, default=2)
    ap.add_argument('--timeout', type=float, default=600)
    ap.add_argument('--mode', default='1600x1000')
    ap.add_argument('--tools', help='directories with labwc, wlr-randr and wtype, PATH-style')
    args = ap.parse_args()
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.regime, args.lines, args.classic, args.state, args.gap, args.populate = 'loadup', 20, False, False, .25, False
    args.output.mkdir(parents=True, exist_ok=True)
    report = {'deadline': args.deadline, 'steps': [], 'failures': []}
    failures = report['failures']
    with tempfile.TemporaryDirectory(prefix='omarchy-live-', dir=os.environ.get('XDG_RUNTIME_DIR')) as tmp:
        root = Path(tmp)
        root.chmod(0o700)
        state, log = root / 'state', root / 'enabled.log'
        state.mkdir()
        switch(state, 'g0', 'rum', PALETTE)
        session = theme_live.Session(args, root, False)
        try:
            session.start()
            session.eval_form(f'''(progn (require 'romacs-omarchy)
               (add-hook 'enable-theme-functions
                 (lambda (theme) (write-region (format "%s %f\\n" theme (float-time)) nil "{log}" t 'quiet)))
               (setq omarchy-state-directory "{state}/") (omarchy-mode 1))''')
            seen = len(log.read_text().splitlines()) if log.exists() else 0
            if session.value('custom-enabled-themes') != '(rum)':
                failures.append(f"turning the mode on showed {session.value('custom-enabled-themes')}")
            shots = 0
            for generation, name, palette, theme in [('g1', 'tokyo-night', PALETTE, 'omarchy-tokyo-night'),
                                                     ('g2', 'an-unshipped-theme', PALETTE, 'omarchy-an-unshipped-theme'),
                                                     ('g3', 'rum', PALETTE, 'rum')]:
                wrote = switch(state, generation, name, palette)
                rows = enabled_after(log, seen, args.deadline + 2)
                seen = len(log.read_text().splitlines())
                row = [r for r in rows if r[0] == theme]
                latency = float(row[0][1]) - wrote if row else None
                answer = session.value('(+ 1 1)')
                step = {'name': name, 'theme': theme, 'latency': latency, 'answered': answer == '2',
                        'enabled': session.value('custom-enabled-themes')}
                report['steps'].append(step)
                if not row:
                    failures.append(f'{name}: {theme} not enabled ({rows})')
                elif latency > args.deadline:
                    failures.append(f'{name}: followed after {latency:.3f} s')
                if step['enabled'] != f'({theme})':
                    failures.append(f"{name}: enabled {step['enabled']}")
                if not step['answered']:
                    failures.append(f'{name}: the session did not answer')
                shots += 1
                subprocess.run(['/usr/bin/grim', str(args.output / f'{shots}-{name}.png')],
                               env=session.environ, check=True, timeout=30)
            # A writer stalled past the delay: the new palette never pairs
            # with the old name, under either theme.
            switch(state, 'g3a', 'an-unshipped-theme', PALETTE)
            enabled_after(log, seen, args.deadline + 2)
            seen = len(log.read_text().splitlines())
            other = PALETTE.replace('#101820', '#301020')
            wrote = staged_switch(state, 'g3b', 'another-unshipped-theme', other, .6)
            rows = enabled_after(log, seen, args.deadline + 2)
            seen = len(log.read_text().splitlines())
            background = session.value("(face-background 'default)")
            kept = session.value("(cdr (assq 'bg (romacs-theme-palette 'omarchy-an-unshipped-theme)))")
            report['staged'] = {'enabled': rows, 'background': background, 'earlier-theme-bg': kept}
            if [r[0] for r in rows] != ['omarchy-another-unshipped-theme']:
                failures.append(f'a stalled writer enabled {rows}')
            if background != '"#301020"' or kept != '"#101820"':
                failures.append(f'a stalled writer showed {background}, left the earlier theme {kept}')
            # A command finishes before the theme changes.
            out = root / 'in-command.el'
            session.eval_form(f'''(progn (write-region "g3c\\n" nil "{state}/theme.generation")
               (write-region "rum\\n" nil "{state}/theme.name") (sit-for 1.2)
               (with-temp-file "{out}" (prin1 custom-enabled-themes (current-buffer))))''')
            report['in-command'] = out.read_text()
            if out.read_text() != '(omarchy-another-unshipped-theme)':
                failures.append(f'the theme changed inside a command: {out.read_text()}')
            enabled_after(log, seen, args.deadline + 2)
            if session.value('custom-enabled-themes') != '(rum)':
                failures.append(f"after the command {session.value('custom-enabled-themes')}")
            # A key in an open prompt is a command inside one: the theme
            # waits for the prompt to end.
            session.eval_form('(ignore)')
            done = root / 'prompt.el'
            session.type(['-M', 'alt', ':', '-m', 'alt'])
            time.sleep(.2)
            session.type([f'(progn (read-string "p: ") (with-temp-file "{done}" (prin1 custom-enabled-themes (current-buffer))))'])
            session.type(['-k', 'Return'])
            time.sleep(.6)
            switch(state, 'g3d', 'tokyo-night', PALETTE)
            time.sleep(args.deadline)
            session.type(['abc'])
            time.sleep(.5)
            session.type(['-k', 'Return'])
            end = time.monotonic() + 10
            while not done.exists() and time.monotonic() < end:
                time.sleep(.05)
            report['in-prompt'] = done.read_text() if done.exists() else None
            if report['in-prompt'] != '(rum)':
                failures.append(f"the theme changed inside a prompt: {report['in-prompt']}")
            enabled_after(log, len(log.read_text().splitlines()), args.deadline + 2)
            if session.value('custom-enabled-themes') != '(omarchy-tokyo-night)':
                failures.append(f"after the prompt {session.value('custom-enabled-themes')}")
            # A theme by hand pauses following; the next change follows.
            session.eval_form("(load-theme 'nord t)")
            seen = len(log.read_text().splitlines())
            paused = session.value('omarchy-mode-paused')
            report['paused'] = paused
            if paused != 't':
                failures.append(f'a theme by hand left paused {paused}')
            wrote = switch(state, 'g4', 'rum', PALETTE)
            rows = enabled_after(log, seen, args.deadline + 2)
            if not [r for r in rows if r[0] == 'rum']:
                failures.append(f'the next change did not follow ({rows})')
            if session.value('omarchy-mode-paused') != 'nil':
                failures.append('the next change left following paused')
        except (perkey.Invalid, AssertionError, OSError) as error:
            failures.append(repr(error))
        finally:
            session.stop()
    report['status'] = 'FAIL' if failures else 'PASS'
    (args.output / 'report.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report))
    return 1 if failures else 0


if __name__ == '__main__':
    raise SystemExit(main())
