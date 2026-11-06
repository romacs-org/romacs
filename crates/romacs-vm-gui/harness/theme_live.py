#!/usr/bin/env python3
"""Theme switch cost in a live GUI session: evaluator instructions per switch.

One fresh private headless labwc and one `romacs-vm-gui -Q --loadup', as
perkey_live.py starts them (private HOME/XDG/TMPDIR, no DISPLAY; keys typed
with wtype into that compositor only).  Each switch is one M-: of
`(theme-live-switch 'THEME LOAD)': every enabled theme disabled, then THEME
enabled, loading its file first when LOAD (`load-theme' THEME t, which
evaluates the file every time, as GNU's does).  The evaluator thread alone
is counted over it (`perf stat -t', instructions:u), from before the M-: to
after its reply and a quiet evaluator, so the redisplay the switch causes
counts too; the Lisp elapsed time of the switch itself is reported beside.
The median of three M-:s that do nothing, counted the same way, is
subtracted: what is reported is the switch, not the M-: around it.

The switches run in rounds: the session's own faces, then after each
--extra step that many more faces (`custom-declare-face', a background-
dependent spec each) are declared.  Every round switches through all of
--themes twice with `load-theme' from a key (reported as `first' and
`repeat'), then with `enable-theme' alone, one M-: per switch (reported per theme), then
--rounds passes of `enable-theme' through them all in one M-:: applying a
loaded theme, per switch, what the budget and the growth check judge.
One M-: costs more than a switch and varies by more, so only the cycled
measure, which divides it among all its switches, is judged.

The run fails when
  * a switch signals, or a theme is not the one enabled after it;
  * a cycled switch of the first round costs more than --budget
    instructions.  Pinned GNU, cycling the same shape (its Rum on
    modus-themes 5.3.0, modus-vivendi, tango-dark; four cycled rounds and a
    redisplay in one command, net of a command that switches nothing, its
    whole process counted, 203 faces), costs 46.0 M per switch, and 78.0 M
    when the no-theme state between themes flips the background mode as
    it does here.  The budget, 43.2 M, stayed from a run in which
    modus-vivendi was never loaded (36.0 M), and is held to;
  * a round's cycled switch grows faster than its face count: its ratio
    to the first round's exceeds --slack times the ratio of the face
    counts (cost linear in the faces passes; quadratic does not);
  * `load-theme' of a theme in LOAD_BUDGETS, first visit or repeat, costs
    more than its budget, net of the key that runs it visiting nothing.
    The stock theme has no counterpart in GNU and is reported only;
  * `C-g' during a long cycle takes more than --quit-limit to end it.

usage: theme_live.py --binary PATH --output DIR [--themes default,modus-vivendi,tango-dark]
                     [--extra 1000] [--budget 43.2e6] [--slack 1.5] [--rounds 40]
                     [--tools DIR:DIR]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import tempfile
import time

import perkey_live as perkey

# 1.2 x pinned GNU's `load-theme' with a redisplay, whole process, from a key
# in a -Q session: modus-vivendi 788 M on a first visit, which reads the
# library, and 311 M to 380 M on a repeat; tango-dark 96 M to disable it and
# load it again.  GNU's -Q has no theme to disable on a first visit, so its
# 16 M there is not this session's first visit; the repeat's budget holds.
LOAD_BUDGETS = {'first': {'modus-vivendi': 946e6, 'tango-dark': 115e6},
                'repeat': {'modus-vivendi': 373e6, 'tango-dark': 115e6}}

SETUP = r'''(progn
  (setq auto-save-timeout nil)
  (defun theme-live-switch (theme load)
    (let ((started (float-time)))
      (mapc #'disable-theme custom-enabled-themes)
      (if load (load-theme theme t) (enable-theme theme))
      (list (- (float-time) started) custom-enabled-themes (length (face-list)))))
  (defun theme-live-cycle (themes rounds)
    (let ((started (float-time)))
      (dotimes (_ rounds)
        (dolist (theme themes)
          (mapc #'disable-theme custom-enabled-themes)
          (enable-theme theme)))
      (list (- (float-time) started) custom-enabled-themes (length (face-list)))))
  (defvar theme-live-queue nil)
  (defvar theme-live-visited nil)
  (defun theme-live-visit ()
    (interactive)
    (let ((theme (pop theme-live-queue)))
      (when theme
        (mapc #'disable-theme custom-enabled-themes)
        (load-theme theme t))
      (write-region (format "%S\n" (car custom-enabled-themes)) nil theme-live-visited t 'silent)))
  (global-set-key (kbd "<f9>") #'theme-live-visit)
  (defun theme-live-noop ()
    (list 0.0 custom-enabled-themes (length (face-list))))
  (defun theme-live-faces (n)
    (let ((base (length (face-list))))
      (dotimes (i n)
        (custom-declare-face (intern (format "theme-live-%d-%d" base i))
                             '((((background dark)) :foreground "#aabbcc") (t :foreground "#112233")) "Probe."))
      (length (face-list))))
  t)'''


class Session(perkey.Session):
    def eval_form(self, form):
        """M-: FORM; its reply records, once the evaluator is quiet again."""
        before = len(perkey.records(self.stdout))
        self.type(['-M', 'alt', ':', '-m', 'alt'])
        time.sleep(.2)
        self.type([form])
        self.type(['-k', 'Return'])
        done = lambda: [r for r in perkey.records(self.stdout)[before:] if r.get('native_command') == 'eval-expression']
        self.wait(done, self.args.timeout, f'M-: {form[:40]}')
        self.quiesce()
        return done()

    def value(self, form):
        """FORM's value, read back through a file the M-: writes."""
        out = self.root / f'value-{time.monotonic_ns()}.el'
        self.eval_form(f'(with-temp-file "{out}" (prin1 {form} (current-buffer)))')
        return out.read_text()


def cycle(session, themes, rounds, noop):
    """ROUNDS passes of `enable-theme' through THEMES in one M-:: per switch,
    the instructions net of NOOP and the Lisp seconds."""
    out = session.root / f'cycle-{time.monotonic_ns()}.el'
    names = ' '.join(themes)
    form = f"(with-temp-file \"{out}\" (prin1 (theme-live-cycle '({names}) {rounds}) (current-buffer)))"
    count, _, replies = session.counted(lambda: session.eval_form(form))
    switches = rounds * len(themes)
    text = out.read_text() if out.exists() else ''
    return {'switches': switches, 'instructions': (count - noop) / switches,
            'lisp_seconds': float(text[1:].split()[0]) / switches if text.startswith('(') else None,
            'problems': [str(r) for r in replies if r.get('signal') or r.get('native_error')]}


def switch(session, theme, load):
    """One switch to THEME (with `load-theme' when LOAD), or with THEME None
    the same M-: doing nothing: what the M-: itself costs."""
    out = session.root / f'switch-{time.monotonic_ns()}.el'
    call = f"(theme-live-switch '{theme} {'t' if load else 'nil'})" if theme else '(theme-live-noop)'
    form = f'(with-temp-file "{out}" (prin1 {call} (current-buffer)))'
    count, seconds, replies = session.counted(lambda: session.eval_form(form))
    problems = [r for r in replies if r.get('signal') or r.get('native_error')]
    text = out.read_text() if out.exists() else ''
    lisp_seconds = float(text[1:].split()[0]) if text.startswith('(') else None
    enabled = text.split('(', 2)[2].split(')')[0].split() if text.count('(') >= 2 else []
    if theme and not problems and enabled[:1] != [theme]:
        problems.append(f'enabled {enabled}, not {theme}')
    return {'theme': theme, 'instructions': count, 'seconds': seconds, 'lisp_seconds': lisp_seconds,
            'problems': [str(p) for p in problems]}


def visits(session, themes):
    """`load-theme' of each of THEMES from a key, net of the same key
    visiting nothing: an M-: varies by more than a load costs."""
    out = session.root / f'visits-{time.monotonic_ns()}.txt'
    session.eval_form(f"(setq theme-live-queue '({' '.join(themes)} nil nil nil) theme-live-visited \"{out}\")")
    lines = lambda: out.read_text().splitlines() if out.exists() else []
    def press():
        before = len(lines())
        def key():
            session.type(['-k', 'F9'])
            session.wait(lambda: len(lines()) > before, session.args.timeout, 'a visit')
            session.quiesce()
            return []
        return session.counted(key)[0]
    measured = [press() for _ in themes]
    noop = statistics.median(press() for _ in range(3))
    return [{'theme': theme, 'net_instructions': count - noop,
             'problems': [] if shown == theme else [f'enabled {shown}']}
            for theme, count, shown in zip(themes, measured, lines())]


def quit_seconds(session, themes):
    """Seconds from `C-g' to the end of a cycle far longer than the test:
    the switch's own loops must take the quit, not its end."""
    names = ' '.join(themes)
    before = len(perkey.records(session.stdout))
    session.type(['-M', 'alt', ':', '-m', 'alt'])
    time.sleep(.2)
    session.type([f"(theme-live-cycle '({names}) 1000000)"])
    session.type(['-k', 'Return'])
    time.sleep(1)
    done = lambda: [r for r in perkey.records(session.stdout)[before:] if r.get('native_command') == 'eval-expression']
    if done():
        raise perkey.Invalid(f'the cycle ended before C-g: {done()}')
    pressed = time.monotonic()
    session.type(['-M', 'ctrl', 'g', '-m', 'ctrl'])
    session.wait(done, session.args.timeout, 'the cycle to quit')
    seconds = time.monotonic() - pressed
    if not any('quit' in str(r.get('signal')) for r in done()):
        raise perkey.Invalid(f'the cycle ended without a quit: {done()}')
    session.quiesce()
    return seconds


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--themes', default='default,modus-vivendi,tango-dark')
    ap.add_argument('--extra', default='1000', help='comma-separated face counts to add between rounds')
    ap.add_argument('--budget', type=float, default=43.2e6, help='instructions per cycled switch at the session\'s faces (1.2 x GNU)')
    ap.add_argument('--slack', type=float, default=1.5)
    ap.add_argument('--rounds', type=int, default=40, help='passes through --themes in the cycled measure: one M-: costs ten switches and varies by more')
    ap.add_argument('--quit-limit', type=float, default=.25, help='seconds from C-g to the end of a long cycle, key delivery included')
    ap.add_argument('--settle', type=float, default=2)
    ap.add_argument('--timeout', type=float, default=600)
    ap.add_argument('--mode', default='1600x1000')
    ap.add_argument('--tools', help='directories with labwc, wlr-randr and wtype, PATH-style')
    args = ap.parse_args()
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.regime, args.lines, args.classic, args.state, args.gap, args.populate, args.split = 'loadup', 20, False, False, .25, False, 1
    args.output.mkdir(parents=True, exist_ok=True)
    themes = [t for t in args.themes.split(',') if t]
    report = {'binary': str(args.binary), 'themes': themes, 'budget': args.budget, 'slack': args.slack, 'rounds': []}
    failures = []
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    with tempfile.TemporaryDirectory(prefix='theme-', dir=runtime) as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        session = Session(args, root, False)
        try:
            session.start()
            session.eval_form(SETUP)
            for extra in [0] + [int(n) for n in args.extra.split(',') if n]:
                faces = int(session.value(f'(theme-live-faces {extra})'))
                loads = [visits(session, themes) for _ in range(2)]
                kept = [switch(session, theme, False) for theme in themes]
                noop = statistics.median(switch(session, None, False)['instructions'] for _ in range(3))
                for s in kept:
                    s['net_instructions'] = s['instructions'] - noop
                cycled = cycle(session, themes, args.rounds, noop)
                median = cycled['instructions']
                report['rounds'].append({'faces': faces, 'median_instructions': median, 'noop_instructions': noop,
                                         'cycle': cycled, 'first': loads[0], 'repeat': loads[1], 'switches': kept})
                failures += [f"cycle: {p}" for p in cycled['problems']]
                print(json.dumps({'faces': faces, 'per_switch': median, 'lisp_seconds': cycled['lisp_seconds'], 'noop': noop,
                                  'first': [(s['theme'], s['net_instructions']) for s in loads[0]],
                                  'repeat': [(s['theme'], s['net_instructions']) for s in loads[1]],
                                  'apply': [(s['theme'], s['net_instructions'], s['lisp_seconds']) for s in kept]}), flush=True)
                failures += [f"{s['theme']}: {s['problems']}" for p in loads + [kept] for s in p if s['problems']]
            report['quit_seconds'] = quit_seconds(session, themes)
            print(json.dumps({'quit_seconds': report['quit_seconds']}), flush=True)
            if report['quit_seconds'] > args.quit_limit:
                failures.append(f"C-g took {report['quit_seconds']:.3f} s to end a cycle, more than {args.quit_limit}")
        except (perkey.Invalid, AssertionError, OSError) as error:
            failures.append(repr(error))
        finally:
            session.stop()
            for name in ('binary.stdout', 'binary.stderr', 'compositor.log'):
                if (root / name).exists():
                    shutil.copy(root / name, args.output / name)
    rounds = report['rounds']
    if rounds:
        first = rounds[0]
        if first['median_instructions'] > args.budget:
            failures.append(f"{first['median_instructions']:.3g} instructions per switch > budget {args.budget:.3g}")
        for name, budgets in LOAD_BUDGETS.items():
            failures += [f"{name} `load-theme' of {s['theme']}: {s['net_instructions']:.3g} > budget {budgets[s['theme']]:.3g}"
                         for s in first[name] if s['net_instructions'] > budgets.get(s['theme'], float('inf'))]
        for r in rounds[1:]:
            growth, allowed = r['median_instructions'] / first['median_instructions'], args.slack * r['faces'] / first['faces']
            r['growth'], r['allowed_growth'] = growth, allowed
            if growth > allowed:
                failures.append(f"{r['faces']} faces: median grew {growth:.2f}x, more than {allowed:.2f}x")
    report['failures'] = failures
    report['status'] = 'FAIL' if failures or not rounds else 'PASS'
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({'status': report['status'], 'failures': failures}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
