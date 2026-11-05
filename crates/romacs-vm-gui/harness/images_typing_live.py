#!/usr/bin/env python3
"""Typing while large images open in another window: key latency.

One fresh private headless labwc session (perkey_live's) runs
`romacs-vm-gui -Q --loadup --trace-latency` on a text file, its frame
split side by side: the left window shows the file, the right a buffer
for the images.  One M-: inserts IMAGES (by default a PNG and an SVG, each
`:max-width' 600) into the right window's buffer and returns; the harness
then at once types --keys `x' keys --gap seconds apart into the left
window while the images decode.  Every key's latency is the session's own
trace, key enqueue to render observed (`latency-key').  After typing, a
second M-: asks `image-size' of each image (a query waits for its
decode) and the report gives the sizes and the time from the insert to
the first paint after it with each image's pixels.

The image files are copied into the session's private directory before
it starts: their first read is the session's (the page cache may already
hold them; the copy is not evicted).

With --hold the session's decodes are held (ROMACS_TEST_IMAGE_HOLD names a
file that keeps every decode from starting while it exists): after typing, the
images' boxes are read from the layout (`posn-at-point', which does not
wait), the file is removed with no further input, and the report says
whether a paint followed.

Reports the redisplay after the M-: insert (where the images are looked up), the typed keys' p50/p99/max, and the
keys over 16 ms.  Exits nonzero when any is over --budget ms.

usage: images_typing_live.py --binary PATH --output DIR --image FILE...
                             [--keys 60] [--gap 0.05] [--budget 16] [--tools DIR:DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import perkey_live as perkey

SPLIT = ('(progn (select-window (split-window-right)) (switch-to-buffer (get-buffer-create "*images*"))'
         ' (other-window 1) t)')


def insert_form(files):
    images = ' '.join(f'(insert-image (create-image {json.dumps(str(f))} nil nil :max-width 600)) (insert "\\n")'
                      for f in files)
    return f'(with-current-buffer "*images*" {images} t)'


def sizes_form(files):
    sizes = ' '.join(f'(image-size (create-image {json.dumps(str(f))} nil nil :max-width 600) t)' for f in files)
    return f'(message "images-typing-sizes %S" (list {sizes}))'


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(fraction * len(ordered)))]


class Session(perkey.Session):
    def env(self):
        env = super().env()
        if self.args.hold:
            env['ROMACS_TEST_IMAGE_HOLD'] = str(self.root / 'hold')
        return env

    def held(self):
        """The images' boxes from the layout while held, then whether
        releasing them, with no input, paints."""
        boxes = self.root / 'boxes.txt'
        self.eval_form('(with-selected-window (get-buffer-window "*images*") (let (boxes) (save-excursion'
                       ' (goto-char (point-min)) (while (not (eobp)) (push (posn-object-width-height (posn-at-point)) boxes)'
                       f' (forward-line 1))) (write-region (format "%S" (nreverse boxes)) nil {json.dumps(str(boxes))})))')
        painted = lambda: [r['presentation'] for r in perkey.records(self.stderr) if r.get('diagnostic') == 'paint']
        time.sleep(1)
        last = max(painted(), default=0)
        (self.root / 'hold').unlink()
        released = time.monotonic()
        try:
            self.wait(lambda: max(painted(), default=0) > last, 60, 'a paint after the release')
            repainted = time.monotonic() - released
        except perkey.Invalid:
            repainted = None
        return {'held_boxes': boxes.read_text(), 'repainted_after_release_s': repainted}

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
        return done()[0]

    def measure(self, files):
        self.eval_form(SPLIT)
        self.quiesce()
        start = len(perkey.records(self.stderr))
        inserted_at = time.monotonic()
        reply = self.eval_form(insert_form(files))
        replies = self.run_keys(['x'], self.args.keys)
        typed_s = time.monotonic() - inserted_at
        samples = [r for r in perkey.records(self.stderr)[start:] if r.get('diagnostic') == 'latency-key']
        held = self.held() if self.args.hold else {}
        key_ms = [s['key_to_render_observed_ns'] / 1e6 for s in samples if s.get('key_to_render_observed_ns') is not None]
        self.eval_form(sizes_form(files))
        messages = self.root / 'messages.txt'
        self.eval_form(f'(with-current-buffer (messages-buffer) (write-region nil nil {json.dumps(str(messages))}))')
        logged = messages.read_text(errors='replace') if messages.exists() else ''
        sizes = next((l for l in logged.splitlines() if l.startswith('images-typing-sizes')), None)
        problems = []
        typed = [r for r in replies if r.get('native_command') == 'self-insert-command']
        if len(typed) != self.args.keys:
            problems.append(f'{len(typed)} self-insert replies for {self.args.keys} keys')
        # The M-: Return's own sample is the first after the form was typed;
        # the typed keys follow it.
        # The M-:'s command time holds the minibuffer read (the form being
        # typed); its redisplay, which looks the images up, is its own.
        insert_ms = samples[0]['snapshot_ns'] / 1e6 if samples else None
        typed_ms = key_ms[1:]
        if len(typed_ms) < self.args.keys:
            problems.append(f'{len(typed_ms)} latency samples for {self.args.keys} keys')
        if self.args.hold and held.get('repainted_after_release_s') is None:
            problems.append('no paint after the release')
        return {**held, 'insert_reply': reply.get('native_command'), 'insert_ms': insert_ms, 'typed_s': typed_s,
                'key_ms': [round(k, 2) for k in typed_ms], 'sizes': sizes,
                'messages': logged[-2000:],
                'summary': {'keys': len(typed_ms), 'p50': percentile(typed_ms, .5) if typed_ms else None,
                            'p99': percentile(typed_ms, .99) if typed_ms else None,
                            'max': max(typed_ms, default=None),
                            'over_budget': sum(1 for k in typed_ms if k > self.args.budget)},
                'problems': problems}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--image', type=Path, action='append', required=True)
    parser.add_argument('--keys', type=int, default=60)
    parser.add_argument('--gap', type=float, default=.05)
    parser.add_argument('--budget', type=float, default=16, help='ms a key may take')
    parser.add_argument('--hold', action='store_true', help='hold the decodes until the keys are typed')
    parser.add_argument('--settle', type=float, default=3)
    parser.add_argument('--timeout', type=float, default=240)
    parser.add_argument('--tools', help='directories with labwc, wlr-randr and wtype, PATH-style')
    parser.add_argument('--mode', default='1600x1000')
    args = parser.parse_args()
    # perkey_live's session options this journey fixes.
    args.regime, args.lines, args.sample, args.state = 'loadup', 200, None, False
    args.classic, args.line_numbers, args.populate, args.split = False, None, False, 1
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.output.mkdir(parents=True)
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    with tempfile.TemporaryDirectory(prefix='imtyping-', dir=runtime) as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        images = root / 'images'
        images.mkdir()
        files = [Path(shutil.copy(f, images / f.name)) for f in args.image]
        if args.hold:
            (root / 'hold').write_text('')
        session = Session(args, root, True)
        try:
            session.start()
            result = session.measure(files)
        except (perkey.Invalid, AssertionError, subprocess.SubprocessError) as error:
            result = {'problems': [repr(error)], 'summary': None}
        finally:
            session.stop()
            for name in ('binary.stdout', 'binary.stderr', 'compositor.log'):
                if (root / name).exists():
                    shutil.copy(root / name, args.output / name)
    report = {'binary': str(args.binary), 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'images': [{'name': f.name, 'bytes': f.stat().st_size} for f in args.image],
              'keys': args.keys, 'gap': args.gap, 'budget_ms': args.budget, **result}
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({k: report.get(k) for k in ('insert_ms', 'summary', 'sizes', 'problems')}, indent=1))
    summary = report.get('summary')
    ok = summary and not report['problems'] and summary['over_budget'] == 0 \
        and (report.get('insert_ms') or 0) <= args.budget
    return 0 if ok else 1


if __name__ == '__main__':
    raise SystemExit(main())
