#!/usr/bin/env python3
"""Typing while large images open in another window: the evaluator's work.

One fresh private headless labwc session (perkey_live's counted session)
runs `romacs-vm-gui -Q --loadup` on a text file, its frame split side by
side: the left window shows the file, the right a buffer for the images.
Twice, an M-: inserts into the right window's buffer and returns, and the
harness at once types --keys `x' keys --gap seconds apart into the left
window: first a control that inserts as many newlines and no image, then
IMAGES (a PNG and an SVG, say, each `:max-width' 600), which decode while
the keys are typed.  perf counts the evaluator thread's instructions from
each M-:'s RET to the paint after its last key and, unless --hold holds
them, on until every decode the span started has landed (the decode
worker and then the evaluator wait), less the session's idle rate
(perkey_live's idle phases).  It counts that thread alone: the decode
worker starts at the first image, inside the images' span, and what it
decodes is not the evaluator's work.  Decoding off the evaluator, the
images' span costs about what the control's does; an image decoded on the
evaluator, or redisplay laying the images out again for every key, adds
its work there.  After typing, a third M-: asks `image-size' of each
image (a query waits for its decode).

Work, not wall time, is the verdict: on a loaded host every key's latency
stretches whatever the evaluator does.  The keys' paints are counted; the
report gives each span's seconds for reference.

Before each span and idle phase the evaluator must be quiet for --settle
seconds, past GNU's undo-auto boundary timer (10 s after the first change
since the last boundary, an M-:'s own minibuffer text among them): that
timer's redisplay landing in one idle phase and not the other made them
disagree.

The image files are copied into the session's private directory before
it starts: their first read is the session's (the page cache may already
hold them; the copy is not evicted).

With --hold the session's decodes are held (ROMACS_TEST_IMAGE_HOLD names a
file that keeps every decode from starting while it exists): after typing, the
images' boxes are read from the layout (`posn-at-point', which does not
wait), the file is removed with no further input, and the report says
whether a paint followed.

Exits nonzero when the images' span costs more than --slack (a fraction)
over the control's, a key went unanswered or signalled, no paint followed
the last key, an image failed to load (its size is GNU's 30x30
placeholder), or the idle phases disagree (work the session went on
doing).

usage: images_typing_live.py --binary PATH --output DIR --image FILE...
                             [--keys 60] [--gap 0.05] [--slack 0.25] [--tools DIR:DIR]
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import perkey_live as perkey

# The cursor stops blinking: its timer's work would land in the idle phases.
SPLIT = ('(progn (blink-cursor-mode -1) (select-window (split-window-right)) (switch-to-buffer (get-buffer-create "*images*"))'
         ' (other-window 1) t)')


def insert_form(files):
    images = ' '.join(f'(insert-image (create-image {json.dumps(str(f))} nil nil :max-width 600)) (insert "\\n")'
                      for f in files)
    return f'(with-current-buffer "*images*" {images} t)'


def sizes_form(files):
    sizes = ' '.join(f'(image-size (create-image {json.dumps(str(f))} nil nil :max-width 600) t)' for f in files)
    return f'(message "images-typing-sizes %S" (list {sizes}))'


# GNU's placeholder for an image that failed to load (`image-size' of one
# with no `:width' or `:height').
PLACEHOLDER = (30, 30)


def size_problems(line, count):
    """Problems with the `images-typing-sizes' LINE of COUNT images: each
    must have decoded, not be GNU's placeholder for a failed load."""
    sizes = [tuple(map(int, pair)) for pair in re.findall(r'\((\d+) \. (\d+)\)', line or '')]
    if len(sizes) != count:
        return [f'{len(sizes)} image sizes for {count} images: {line!r}']
    return [f'image {i + 1} failed to load (the placeholder\'s size)' for i, size in enumerate(sizes) if size == PLACEHOLDER]


def thread_state(stat):
    """(STATE, CPU ticks) of a thread from its /proc stat text."""
    fields = stat.rsplit(')', 1)[1].split()
    return fields[0], int(fields[11]) + int(fields[12])


def waiting(before, after):
    """Whether threads read as BEFORE and AFTER, [(STATE, ticks)] each,
    waited all the while: asleep at both readings, with no CPU time between."""
    return len(before) == len(after) and all(b[0] == a[0] == 'S' and b[1] == a[1] for b, a in zip(before, after))


def control_form(files):
    lines = ' '.join('(insert "\\n")' for _ in files)
    return f'(with-current-buffer "*images*" {lines} t)'


class Session(perkey.Session):
    # The evaluator thread alone: the decode worker starts at the first
    # image, inside the images' span, and its decoding is not the
    # evaluator's work.
    inherit = False

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
        return self.read_form(form)()

    def read_form(self, form):
        """Type M-: FORM, all but its RET: a function that types the RET and
        returns the M-:'s reply once it has run."""
        before = len(perkey.records(self.stdout))
        self.type(['-M', 'alt', ':', '-m', 'alt'])
        time.sleep(.2)
        self.type([form])

        def finish():
            self.type(['-k', 'Return'])
            done = lambda: [r for r in perkey.records(self.stdout)[before:] if r.get('native_command') == 'eval-expression']
            self.wait(done, self.args.timeout, f'M-: {form[:40]}')
            if any(r.get('signal') or r.get('native_error') for r in done()):
                raise perkey.Invalid(f'M-: {form[:60]} signalled: {done()}')
            return done()[0]
        return finish

    def span(self, form):
        """The evaluator's instructions and seconds from FORM's RET to the
        paint after --keys typed keys, its reply, the keys' replies and paints.
        Unless --hold holds them, the span goes on until every decode it
        started has landed (`landed').  The heap is collected first: a
        collection due by the session's allocation so far would land in
        whichever span crossed its threshold."""
        self.eval_form('(garbage-collect)')
        finish = self.read_form(form)
        self.quiesce()
        start = len(perkey.records(self.stderr))

        def typed():
            reply, replies = finish(), self.run_keys(['x'], self.args.keys)
            if not self.args.hold:
                self.landed()
            return reply, replies
        count, seconds, (reply, replies) = self.counted(typed)
        paints = sum(1 for r in perkey.records(self.stderr)[start:] if r.get('diagnostic') == 'paint')
        return {'instructions': count, 'seconds': round(seconds, 2), 'reply': reply.get('native_command'),
                'paints': paints, 'problems': perkey.delivery(replies, 'self-insert-command', self.args.keys)}

    def threads(self, name):
        """(STATE, CPU ticks) of each of the binary's threads called NAME."""
        found = []
        for task in sorted(Path(f'/proc/{self.process.pid}/task').iterdir()):
            try:
                if (task / 'comm').read_text().strip() == name:
                    found.append(thread_state((task / 'stat').read_text()))
            except OSError:
                continue
        return found

    def landed(self):
        """Wait until the decodes are done and landed: the decode worker
        (none before the first image) and then the evaluator wait, asleep
        with no CPU time over --landed seconds.  The worker wakes the
        evaluator with each result, which lands it at once."""
        end = time.monotonic() + self.args.timeout
        evaluator = lambda: [thread_state(Path(f'/proc/{self.process.pid}/task/{self.tid}/stat').read_text())]
        for readings in (lambda: self.threads('image-decode'), evaluator):
            while True:
                if time.monotonic() > end:
                    raise perkey.Invalid('the decodes never finished')
                before = readings()
                time.sleep(self.args.landed)
                if waiting(before, readings()):
                    break

    def idle(self):
        self.quiesce()
        count, seconds, _ = self.counted(lambda: time.sleep(self.args.keys * self.args.gap), idle=True)
        return count, seconds

    def measure(self, files):
        self.eval_form(SPLIT)
        idles = [self.idle()]
        control = self.span(control_form(files))
        images = self.span(insert_form(files))
        held = self.held() if self.args.hold else {}
        # The query waits for every decode: none lands in the idle phase.
        self.eval_form(sizes_form(files))
        idles.append(self.idle())
        rate = perkey.idle_rate(idles)
        for span in (control, images):
            span['per_key'] = round((span['instructions'] - rate * span['seconds']) / self.args.keys)
        messages = self.root / 'messages.txt'
        self.eval_form(f'(with-current-buffer (messages-buffer) (write-region nil nil {json.dumps(str(messages))}))')
        logged = messages.read_text(errors='replace') if messages.exists() else ''
        sizes = next((l for l in logged.splitlines() if l.startswith('images-typing-sizes')), None)
        problems = [f'{name}: {problem}' for name, span in (('control', control), ('images', images))
                    for problem in span.pop('problems')]
        problems += size_problems(sizes, len(files))
        if images['per_key'] > control['per_key'] * (1 + self.args.slack):
            problems.append(f"the images' span cost {images['per_key']} instructions a key, "
                            f"over {1 + self.args.slack:.2f} times the control's {control['per_key']}")
        if self.args.hold and held.get('repainted_after_release_s') is None:
            problems.append('no paint after the release')
        return {**held, 'idle_rate': rate, 'control': control, 'images': images, 'sizes': sizes,
                'messages': logged[-2000:], 'problems': problems}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--image', type=Path, action='append', required=True)
    parser.add_argument('--keys', type=int, default=60)
    parser.add_argument('--gap', type=float, default=.05)
    parser.add_argument('--slack', type=float, default=.25,
                        help="the fraction the images' span may cost over the control's, per key")
    parser.add_argument('--hold', action='store_true', help='hold the decodes until the keys are typed')
    # GNU's undo-auto--boundary-timer fires 10s after a change.
    parser.add_argument('--settle', type=float, default=12, help='quiet evaluator seconds before each span and idle phase')
    parser.add_argument('--landed', type=float, default=.5,
                        help='seconds the decode worker and the evaluator must wait for the decodes to count as landed')
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
        session = Session(args, root, False)
        try:
            session.start()
            result = session.measure(files)
        except (perkey.Invalid, AssertionError, subprocess.SubprocessError) as error:
            result = {'problems': [repr(error)]}
        finally:
            session.stop()
            for name in ('binary.stdout', 'binary.stderr', 'compositor.log'):
                if (root / name).exists():
                    shutil.copy(root / name, args.output / name)
    report = {'binary': str(args.binary), 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'images': [{'name': f.name, 'bytes': f.stat().st_size} for f in args.image],
              'keys': args.keys, 'gap': args.gap, 'slack': args.slack, 'load': os.getloadavg(), **result}
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({k: report.get(k) for k in ('control', 'images', 'sizes', 'problems')}, indent=1))
    return 0 if 'images' in report and not report['problems'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
