#!/usr/bin/env python3
"""Attachments' tray, edit mode and viewer with real keys (attachments.md A1).

One fresh private headless labwc session (perkey_live's) runs
`romacs-vm-gui -Q --loadup --trace-latency`.  Its M-: opens the test host
(`romacs-attach-test-host') and installs a `post-command-hook' that logs,
after every command in it, the command, the items' names and captions,
the selected chip, the viewed item and the draft.  Then:

  attach   M-: attaches plain.png, photo.jpg and a text file in one batch;
           a capture of the chips (thumbnails, icon, labels, sizes).
  edit     DEL at the draft's start selects the last chip; b; M-b moves
           it left; d removes it, u brings it back; c captions it
           (`hello' RET); a capture of the selected chip.
  view     RET views the selected chip (siblings the tray's items); n
           shows the next; + zooms; captures; q closes; q leaves edit
           mode; `hi' types into the draft.
  icons    M-: attaches dot.xpm, a PBM and an SVG without dimensions,
           which get icons, not thumbnails; then --keys `x' keys --gap
           apart, each key's latency from the session's trace.

The report holds the log's states after each step and the checks on
them, the capture paths, and the typing latency.  Exits nonzero when a
check fails or a key passes --budget ms.

usage: attach_live.py --binary PATH --output DIR [--keys 30] [--gap 0.08]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import perkey_live as perkey

FIXTURES = Path(__file__).resolve().parent / 'fixtures' / 'images'

LOGGER = """(setq debug-on-error nil)
(run-with-timer 1 1 (lambda () (with-current-buffer (messages-buffer)
                                 (write-region nil nil {messages} nil 'silent))))
(defvar attach-journey-logging t)
(romacs-attach-test-host)
(add-hook 'post-command-hook
          (lambda ()
            (when (and attach-journey-logging (derived-mode-p 'romacs-attach-test-host-mode))
              (let ((name (lambda (i) (plist-get i :name))))
                (write-region
                 (format "%S\\n"
                         (list this-command
                               (mapcar name romacs-attach-items)
                               (mapcar (lambda (i) (plist-get i :caption)) romacs-attach-items)
                               (and romacs-attach--selected
                                    (funcall name (seq-find (lambda (i) (equal (plist-get i :id) romacs-attach--selected))
                                                            romacs-attach-items)))
                               (and romacs-attach--view
                                    (funcall name (romacs-attach--thing (plist-get romacs-attach--view :thing))))
                               (buffer-substring-no-properties (point-min) (point-max))))
                 nil {log} t 'silent)))))
"""


def lisp_string(path):
    return json.dumps(str(path))


class Session(perkey.Session):
    def eval_form(self, form):
        before = len(perkey.records(self.stdout))
        self.type(['-M', 'alt', ':', '-m', 'alt'])
        time.sleep(.2)
        self.type([form])
        self.type(['-k', 'Return'])
        done = lambda: [r for r in perkey.records(self.stdout)[before:] if r.get('native_command') == 'eval-expression']
        self.wait(done, self.args.timeout, f'M-: {form[:40]}')
        if any(r.get('signal') or r.get('native_error') for r in done()):
            raise perkey.Invalid(f'M-: {form[:60]} signalled: {done()}')

    def states(self):
        if not self.log.exists():
            return []
        return [line for line in self.log.read_text().splitlines() if line.strip()]

    def key(self, parts, what):
        """Type PARTS and wait for the logged command after it."""
        before = len(self.states())
        self.type(parts)
        self.wait(lambda: len(self.states()) > before, 30, what)
        time.sleep(.15)
        return self.states()[-1]

    def capture(self, name):
        time.sleep(1.0)
        shot = self.args.output / f'{name}.png'
        subprocess.run([shutil.which('grim') or '/usr/bin/grim', str(shot)], env=self.environ, check=True, timeout=30)
        return str(shot)

    def typing(self):
        """--keys `x' keys --gap apart with the state log off: each key's
        latency from the session's trace."""
        self.eval_form('(setq attach-journey-logging nil)')
        time.sleep(1)
        start = len(perkey.records(self.stderr))
        for _ in range(self.args.keys):
            self.type(['x'])
            time.sleep(self.args.gap)
        time.sleep(1)
        self.eval_form('(setq attach-journey-logging t)')
        return [r['key_to_render_observed_ns'] / 1e6 for r in perkey.records(self.stderr)[start:]
                if r.get('diagnostic') == 'latency-key' and r.get('key_to_render_observed_ns') is not None][:self.args.keys]

    def journey(self):
        self.log = self.root / 'states.eld'
        text = self.root / 'work' / 'notes.txt'
        text.write_text('notes for the agent\n')
        pbm = self.root / 'work' / 'dot.pbm'
        pbm.write_text('P1\n2 2\n1 0\n0 1\n')
        unsized = self.root / 'work' / 'unsized.svg'
        unsized.write_text('<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="4"/></svg>')
        journey = self.root / 'work' / 'attach-journey.el'
        journey.write_text(LOGGER.format(log=lisp_string(self.log), messages=lisp_string(self.root / 'messages.txt')))
        error = self.root / 'journey-error'
        self.eval_form(f'(condition-case e (load {lisp_string(journey)} nil t) '
                       f'(error (write-region (format "%S" e) nil {lisp_string(error)}) nil))')
        if error.exists():
            raise perkey.Invalid(f'the journey did not load: {error.read_text()}')
        steps, shots = {}, {}
        baseline_ms = self.typing()
        self.eval_form('(with-current-buffer "*attach test*" (erase-buffer) t)')
        files = ' '.join(lisp_string(p) for p in (FIXTURES / 'plain.png', FIXTURES / 'photo.jpg', text))
        self.eval_form(f'(with-current-buffer "*attach test*" (romacs-attach-files (list {files})) t)')
        shots['chips'] = self.capture('chips')
        steps['DEL'] = self.key(['-k', 'BackSpace'], 'DEL')
        steps['b'] = self.key(['b'], 'b')
        steps['M-b'] = self.key(['-M', 'alt', 'b', '-m', 'alt'], 'M-b')
        steps['d'] = self.key(['d'], 'd')
        steps['u'] = self.key(['u'], 'u')
        self.type(['c'])
        time.sleep(.5)
        self.type(['hello'])
        steps['c hello RET'] = self.key(['-k', 'Return'], 'caption')
        shots['selected'] = self.capture('selected')
        steps['RET'] = self.key(['-k', 'Return'], 'RET')
        shots['viewer'] = self.capture('viewer')
        steps['n'] = self.key(['n'], 'n')
        steps['+'] = self.key(['+'], '+')
        shots['viewer-zoom'] = self.capture('viewer-zoom')
        steps['q'] = self.key(['q'], 'q')
        steps['q again'] = self.key(['q'], 'q again')
        steps['h'] = self.key(['h'], 'h')
        steps['i'] = self.key(['i'], 'i')
        icons = ' '.join(lisp_string(p) for p in (FIXTURES / 'dot.xpm', pbm, unsized))
        self.eval_form(f'(with-current-buffer "*attach test*" (romacs-attach-files (list {icons})) t)')
        shots['icons'] = self.capture('icons')
        return steps, shots, baseline_ms, self.typing()


def checks(steps):
    expect = {
        'DEL': ('"notes.txt"', 'selected notes.txt'),
        'b': ('"photo.jpg" nil', 'selected photo.jpg'),
        'M-b': ('("photo.jpg" "plain.png" "notes.txt")', 'photo.jpg moved left'),
        'd': ('("plain.png" "notes.txt")', 'removed'),
        'u': ('("photo.jpg" "plain.png" "notes.txt")', 'back'),
        'c hello RET': ('("hello" nil nil)', 'captioned'),
        'RET': (' "photo.jpg" "photo.jpg" ', 'viewing the selected chip'),
        'n': (' "plain.png" ', 'the next sibling'),
        'q again': ('romacs-attach-edit-quit', 'edit mode left'),
        'i': ('"hi")', 'typing reaches the draft'),
    }
    out = []
    for step, (needle, what) in expect.items():
        state = steps.get(step, '')
        out.append({'step': step, 'what': what, 'ok': needle in state, 'state': state})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--keys', type=int, default=30)
    parser.add_argument('--gap', type=float, default=.08)
    parser.add_argument('--budget', type=float, default=16)
    parser.add_argument('--settle', type=float, default=3)
    parser.add_argument('--timeout', type=float, default=240)
    parser.add_argument('--tools', help='directories with labwc, wlr-randr and wtype, PATH-style')
    parser.add_argument('--mode', default='1600x1000')
    args = parser.parse_args()
    args.regime, args.lines, args.sample, args.state, args.gap_keys = 'loadup', 200, None, False, args.gap
    args.classic, args.line_numbers, args.populate, args.split = False, None, False, 1
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.output.mkdir(parents=True)
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    with tempfile.TemporaryDirectory(prefix='attach-', dir=runtime) as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        session = Session(args, root, True)
        try:
            session.start()
            steps, shots, baseline_ms, key_ms = session.journey()
            summary = lambda ms: {'keys': len(ms), 'p50': sorted(ms)[len(ms) // 2] if ms else None, 'max': max(ms, default=None)}
            result = {'steps': steps, 'checks': checks(steps), 'shots': shots, 'key_ms': [round(k, 2) for k in key_ms],
                      'no_chips': summary(baseline_ms), 'six_chips': summary(key_ms),
                      'max_ms': max(key_ms, default=None), 'over_budget': sum(1 for k in key_ms if k > args.budget),
                      'problems': []}
        except (perkey.Invalid, AssertionError, subprocess.SubprocessError) as error:
            result = {'problems': [repr(error)]}
        finally:
            session.stop()
            for name in ('binary.stdout', 'binary.stderr', 'compositor.log', 'states.eld', 'messages.txt'):
                if (root / name).exists():
                    shutil.copy(root / name, args.output / name)
    (args.output / 'report.json').write_text(json.dumps(result, indent=1) + '\n')
    print(json.dumps({k: result.get(k) for k in ('checks', 'no_chips', 'six_chips', 'over_budget', 'problems')}, indent=1))
    ok = not result['problems'] and all(c['ok'] for c in result['checks']) and result['over_budget'] == 0
    return 0 if ok else 1


if __name__ == '__main__':
    raise SystemExit(main())
