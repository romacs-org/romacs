#!/usr/bin/env python3
"""`load-theme''s preview in a live GUI session.

One fresh private headless labwc and one `romacs-vm-gui -Q --loadup', as
theme_live.py starts them (private HOME/XDG/TMPDIR; keys typed with wtype
into that compositor only).  Real keys go through `M-x load-theme':

  * opening the prompt changes nothing;
  * typing a theme's name shows it, in the session's state and on screen;
  * another name shows that one instead;
  * a theme `custom-safe-themes' would ask about is not read;
  * `C-g' leaves the session as it was, pixel for pixel;
  * RET loads what was shown, and the stock theme gives way as it does to
    `load-theme' without a preview.

A candidate costs the themes under it brought back and then itself, two
switches of the stock theme's size, so the instructions per candidate
between two loaded themes are judged against a frame's worth, by one M-:
over many previews, net of the same M-: over none.

usage: theme_preview_live.py --binary PATH --output DIR [--budget 120e6]
                             [--tools DIR:DIR]
"""
import argparse
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

import theme_live

SETUP = r'''(progn
  (setq auto-save-timeout nil)
  (blink-cursor-mode -1)
  (defvar theme-preview-log nil)
  (defun theme-preview-state ()
    (interactive)
    (write-region (format "%S\n" (list custom-enabled-themes (and (bound-and-true-p romacs-theme--stock) t)
                                       (frame-parameter nil 'background-mode) (face-background 'default)
                                       (boundp 'theme-preview-ran)))
                  nil theme-preview-log t 'silent))
  (global-set-key (kbd "<f9>") #'theme-preview-state)
  (defun theme-preview-cycle (rounds)
    (let* ((romacs-theme-preview--base custom-enabled-themes)
           (romacs-theme--stock romacs-theme--stock)
           (romacs-theme-preview--stock romacs-theme--stock))
      (dotimes (_ rounds)
        (romacs-theme-preview--show 'tango-dark)
        (romacs-theme-preview--show 'modus-operandi))
      (when (> rounds 0) (romacs-theme-preview--show nil))))
  t)'''

UNSAFE = '''(deftheme theme-preview-unsafe "Nobody agreed to this.")
(defvar theme-preview-ran t)
(provide-theme 'theme-preview-unsafe)
'''


class Journey:
    def __init__(self, session, out):
        self.session, self.out = session, out
        self.log = session.root / 'preview-state.txt'

    def key(self, *parts):
        self.session.type(list(parts))
        time.sleep(.4)

    def ctrl(self, letter):
        self.key('-M', 'ctrl', letter, '-m', 'ctrl')

    def state(self):
        """The session's state, from a key that works in the minibuffer too."""
        lines = lambda: self.log.read_text().splitlines() if self.log.exists() else []
        before = len(lines())
        self.key('-k', 'F9')
        self.session.wait(lambda: len(lines()) > before, self.session.args.timeout, 'the state')
        return lines()[-1]

    def shot(self, name):
        path = self.out / f'{name}.png'
        time.sleep(1)
        subprocess.run(['/usr/bin/grim', str(path)], env=self.session.environ, check=True, timeout=30)
        return path

    def prompt(self):
        self.key('-M', 'alt', 'x', '-m', 'alt')
        self.key('load-theme')
        self.key('-k', 'Return')
        time.sleep(1)

    def choose(self, name):
        """Replace the input with NAME."""
        self.ctrl('a')
        self.ctrl('k')
        self.key(name)
        time.sleep(.5)


def rgb(path, magick):
    return subprocess.check_output([magick, str(path), '-alpha', 'off', '-depth', '8', 'RGB:-'], timeout=30)


def differing(first, second, magick):
    a, b = rgb(first, magick), rgb(second, magick)
    return abs(len(a) - len(b)) + sum(a[i:i + 3] != b[i:i + 3] for i in range(0, min(len(a), len(b)), 3))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--budget', type=float, default=120e6, help='instructions per candidate: about 13 ms here, under a frame at 60 Hz')
    ap.add_argument('--rounds', type=int, default=20, help='two previews each; two digits, as the M-: over none is typed the same')
    ap.add_argument('--settle', type=float, default=2)
    ap.add_argument('--timeout', type=float, default=600)
    ap.add_argument('--mode', default='1600x1000')
    ap.add_argument('--tools', help='directories with labwc, wlr-randr and wtype, PATH-style')
    ap.add_argument('--magick', default='/usr/bin/magick')
    args = ap.parse_args()
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.regime, args.lines, args.classic, args.state, args.gap, args.populate, args.split = 'loadup', 20, False, False, .25, False, 1
    args.output.mkdir(parents=True, exist_ok=True)
    report, failures = {'binary': str(args.binary)}, []

    def check(ok, what):
        if not ok:
            failures.append(what)

    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    with tempfile.TemporaryDirectory(prefix='preview-', dir=runtime) as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        session = theme_live.Session(args, root, False)
        try:
            session.start()
            themes = root / 'themes'
            themes.mkdir()
            (themes / 'theme-preview-unsafe-theme.el').write_text(UNSAFE)
            session.eval_form(SETUP)
            journey = Journey(session, args.output)
            session.eval_form(f"(setq theme-preview-log \"{journey.log}\" custom-theme-load-path (cons \"{themes}\" custom-theme-load-path))")
            journey.ctrl('l')
            base = journey.state()
            before = journey.shot('before')

            journey.prompt()
            opened = journey.state()
            journey.shot('open')
            journey.choose('tango-dark')
            tango = journey.state()
            journey.shot('tango-dark')
            journey.choose('modus-operandi')
            modus = journey.state()
            journey.shot('modus-operandi')
            journey.choose('theme-preview-unsafe')
            unsafe = journey.state()
            journey.ctrl('g')
            quit_state = journey.state()
            journey.ctrl('l')
            journey.state()
            after = journey.shot('after')
            pixels = differing(before, after, args.magick)

            journey.prompt()
            journey.choose('tango-dark')
            journey.key('-k', 'Return')
            time.sleep(1)
            chosen = journey.state()
            journey.shot('chosen')
            session.eval_form("(progn (mapc #'disable-theme custom-enabled-themes) (load-theme 'modus-operandi t) (enable-theme 'default))")

            report.update(base=base, opened=opened, tango=tango, modus=modus, unsafe=unsafe, quit=quit_state,
                          chosen=chosen, pixels_after_quit=pixels)
            check(opened == base, f'opening the prompt changed {base} to {opened}')
            check('(tango-dark' in tango and '"#2e3436"' in tango, f'tango-dark not shown: {tango}')
            check('(modus-operandi' in modus and 'light' in modus and 'tango-dark' not in modus, f'modus-operandi not shown: {modus}')
            check('theme-preview-unsafe' not in unsafe and unsafe.endswith('nil)'), f'an unsafe theme was read: {unsafe}')
            check(quit_state == base, f'C-g left {quit_state}, not {base}')
            check(pixels == 0, f'{pixels} pixels differ after C-g')
            check('(tango-dark)' in chosen, f'RET left {chosen}')

            def cycle(rounds):
                form = f'(theme-preview-cycle {rounds:02d})'
                return session.counted(lambda: session.eval_form(form))[0]
            cycle(args.rounds)
            cost = (cycle(args.rounds) - min(cycle(0) for _ in range(3))) / (2 * args.rounds)
            report['per_candidate'] = cost
            check(cost <= args.budget, f'{cost:.3g} instructions per candidate > budget {args.budget:.3g}')
        except (theme_live.perkey.Invalid, AssertionError, OSError, subprocess.SubprocessError) as error:
            failures.append(repr(error))
        finally:
            session.stop()
            for name in ('binary.stdout', 'binary.stderr', 'compositor.log'):
                if (root / name).exists():
                    (args.output / name).write_bytes((root / name).read_bytes())
    report['failures'] = failures
    report['status'] = 'FAIL' if failures else 'PASS'
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps(report))
    return 0 if not failures else 1


if __name__ == '__main__':
    raise SystemExit(main())
