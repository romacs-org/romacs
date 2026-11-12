#!/usr/bin/env python3
"""Scrolling a large file while a worker isolate fontifies it ahead.

Each session is a fresh private headless labwc and a fresh `--loadup`
launch (perkey_live.Session), which visits a private copy of FILE through
`M-: (find-file COPY)` once the variant is set: `isolate' (`romacs-fontify-isolate' t:
lisp/romacs/romacs-fontify.el snapshots the buffer and a worker fontifies it,
streaming its properties) or `session' (nil: jit-lock fontifies each
shown chunk in the session, as GNU's redisplay does).  After --wait
seconds the harness types --keys F9 keys --gap seconds apart, each moving
point 50 lines (redisplay scrolls to it and fontifies what it shows).

Lisp hooks stamp, with `float-time', every `jit-lock-fontify-now' call and
its duration (the session's own fontification cost, or the cost of putting
the worker's properties), each block the worker delivers and the job's
end.  Latency sessions run with --trace-latency (key enqueue to render
observed per key); counted sessions count the evaluator thread's
instructions (`perf stat -t', instructions:u) over the F9 phase.  At the
end each session writes the text properties of the buffer up to the
window's end (all but `fontified' and cc-mode's scan caches, keys sorted)
and where it is fontified: every position fontified in both variants must
have the same properties in both.  (Which stretches between the screens
get fontified differs: font-lock reports bounds extended to whole lines,
the worker's properties are put on the chunk asked for.)

usage: fontify_isolate_live.py --binary PATH --output DIR [--file F ...]
                               [--keys 30] [--gap 0.8] [--wait 3] [--tools DIR:DIR]
"""
import argparse
import json
import re
import shutil
from pathlib import Path
import statistics
import tempfile
import time

import perkey_live as perkey

ROOT = Path(__file__).resolve().parents[3]
FILES = [ROOT / 'test/gnu/test/manual/etags/c-src/emacs/src/keyboard.c', ROOT / 'lisp/gnu/subr.el']

# F9 moves point 50 lines: redisplay scrolls to show it and fontifies what
# comes into view.  (Romacs's `scroll-up-command' refuses text with form
# feeds, which subr.el has.)  The form is typed at M-:, so it has no comments.
HOOKS = """(progn
  (defvar romacs-journey-log %s)
  (defun romacs-journey-stamp (kind &rest data)
    (let ((inhibit-message t))
      (write-region (format "%%s %%.6f %%S\\n" kind (float-time) data) nil romacs-journey-log t 'silent)))
  (advice-add 'jit-lock-fontify-now :around
              (lambda (f &optional beg end)
                (let ((t0 (float-time)))
                  (prog1 (funcall f beg end)
                    (romacs-journey-stamp 'jit beg end (- (float-time) t0)
                                          (and romacs-fontify--state (romacs-fontify--covering beg (min (or end (point-max)) (point-max))) t))))))
  (advice-add 'romacs-fontify--keep :after (lambda (_state message) (romacs-journey-stamp 'block (nth 3 message) (nth 4 message))))
  (advice-add 'romacs-fontify--done :after (lambda (&rest _) (romacs-journey-stamp 'done)))
  (advice-add 'isolate-kill :around (lambda (f &rest args) (let ((t0 (float-time))) (prog1 (apply f args) (romacs-journey-stamp 'kill (- (float-time) t0))))))
  (advice-add 'romacs-fontify--start :around (lambda (f &rest args) (let ((t0 (float-time))) (prog1 (apply f args) (romacs-journey-stamp 'start (and romacs-fontify--state t) (- (float-time) t0))))))
  (advice-add 'romacs-fontify--keep :around (lambda (f &rest args) (let ((t0 (float-time))) (prog1 (apply f args) (romacs-journey-stamp 'kept (- (float-time) t0))))))
  (defun romacs-journey-dump (file)
    (let ((pos (point-min)) runs spans (end (window-end nil t)))
      (while (< pos end)
        (let ((next (next-single-property-change pos 'fontified nil end)))
          (when (eq (get-text-property pos 'fontified) t) (push (list pos next) spans))
          (setq pos next)))
      (setq pos (point-min))
      (while (< pos end)
        (let ((next (next-property-change pos nil end)) (props (text-properties-at pos)) kept)
          (while props
            (unless (memq (car props) '(fontified c-in-sws c-is-sws)) (push (cons (car props) (nth 1 props)) kept))
            (setq props (nthcdr 2 props)))
          (setq kept (sort kept (lambda (a b) (string< (car a) (car b)))))
          (when kept (push (list pos next kept) runs))
          (setq pos next)))
      (with-temp-file file
        (let ((print-length nil) (print-level nil))
          (prin1 (nreverse spans) (current-buffer)) (insert "\\n")
          (dolist (r (nreverse runs)) (prin1 (list (nth 0 r) (nth 1 r) (prin1-to-string (nth 2 r))) (current-buffer)) (insert "\\n"))))
      (romacs-journey-stamp 'dumped (length runs) end
                            (and romacs-fontify--state (length (delq nil (append (aref romacs-fontify--state 4) nil)))))))
  (defun romacs-journey-down () (interactive) (forward-line 50))
  (global-set-key [f9] #'romacs-journey-down)
  (setq auto-save-timeout nil isolate-pool-size 2 romacs-fontify-isolate %s)
  t)"""

DOWN = ['-k', 'F9']


def stamps(path):
    found = []
    if path.exists():
        for line in path.read_text(errors='replace').splitlines():
            kind, at, data = line.split(' ', 2)
            found.append((kind, float(at), data))
    return found


class Session(perkey.Session):
    def __init__(self, args, root, latency, variant, file):
        super().__init__(args, root, latency)
        self.variant, self.target = variant, file
        self.log = root / 'journey.log'

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

    def journey(self):
        # A private copy: the typing phase leaves the buffer modified, and
        # the session may auto-save it beside the file it visits.
        copy = self.root / 'work' / self.target.name
        shutil.copyfile(self.target, copy)
        self.eval_form(HOOKS % (json.dumps(str(self.log)), 't' if self.variant == 'isolate' else 'nil'))
        visited = time.time()
        self.eval_form(f'(find-file {json.dumps(str(copy))})')
        time.sleep(self.args.wait)
        start = len(perkey.records(self.stderr))
        count, seconds, replies = self.counted(lambda: self.run_keys(DOWN, self.args.keys))
        new = perkey.records(self.stderr)[start:]
        samples = [r for r in new if r.get('diagnostic') == 'latency-key']
        latency = [s['key_to_render_observed_ns'] / 1e6 for s in samples if s.get('key_to_render_observed_ns') is not None]
        scrolled = time.time()
        self.eval_form(f'(romacs-journey-dump {json.dumps(str(self.root / "runs.eld"))})')
        if self.variant == 'isolate':
            self.wait(lambda: any(k == 'done' for k, _, _ in stamps(self.log)), 600, 'the worker to finish')
        if self.variant == 'isolate':
            time.sleep(1)
        workers = worker_memory(self.process.pid)
        # Typing drops the results; the buffer is snapshotted again once idle.
        start = len(perkey.records(self.stderr))
        _, _, typed = self.counted(lambda: self.run_keys(['x'], 5))
        typed_ms = [s['key_to_render_observed_ns'] / 1e6 for s in perkey.records(self.stderr)[start:]
                    if s.get('diagnostic') == 'latency-key' and s.get('key_to_render_observed_ns') is not None]
        edited = time.time()
        time.sleep(5)
        found = stamps(self.log)
        jit = [(at, eval_data(data)) for kind, at, data in found if kind == 'jit' and at >= scrolled - seconds - 1 and at <= scrolled]
        return {'variant': self.variant, 'file': str(self.target), 'latency': self.latency,
                'instructions': count, 'phase_seconds': seconds,
                'key_ms': latency, 'key_samples': len(samples), 'replies': len(replies),
                'jit_calls': len(jit), 'jit_seconds': sum(d[2] for _, d in jit),
                'jit_covered': sum(1 for _, d in jit if d[3]),
                'jit_max_ms': max((d[2] for _, d in jit), default=0) * 1000,
                'blocks': [(at - visited, data) for kind, at, data in found if kind == 'block'],
                'started': [(at - visited, data) for kind, at, data in found if kind == 'start'],
                'done_after': next((at - visited for kind, at, _ in found if kind == 'done'), None),
                'dumped': next((data for kind, _, data in found if kind == 'dumped'), None),
                'workers': workers, 'typed_ms': typed_ms,
                'restarted': [(at - edited, data) for kind, at, data in found if kind == 'start' and at > edited],
                'blocks_after_edit': sum(1 for kind, at, _ in found if kind == 'block' and at > edited),
                'kill_ms': [float(d.strip('()')) * 1000 for k, _, d in found if k == 'kill'],
                'keep_ms_max': max((float(d.strip('()')) * 1000 for k, _, d in found if k == 'kept'), default=0),
                'runs': (self.root / 'runs.eld').read_text()}


def worker_memory(parent):
    """VmRSS, VmHWM, RssAnon, RssFile and RssShmem in MiB of PARENT's worker isolates (children run with --isolate)."""
    found = []
    for proc in Path('/proc').iterdir():
        try:
            if not proc.name.isdigit() or b'--isolate' not in (proc / 'cmdline').read_bytes():
                continue
            status = dict(line.split(':', 1) for line in (proc / 'status').read_text().splitlines() if ':' in line)
            if int(status['PPid']) == parent:
                found.append({k: int(status[k].split()[0]) // 1024 for k in ('VmRSS', 'VmHWM', 'RssAnon', 'RssFile', 'RssShmem')})
        except (OSError, KeyError, ValueError):
            pass
    return found


def parse_dump(text):
    """(fontified spans, runs) of a dump: spans [(beg, end)], runs [(beg, end, printed plist)]."""
    lines = text.splitlines()
    spans = [(int(beg), int(end)) for beg, end in re.findall(r'\((\d+) (\d+)\)', lines[0])]
    runs = []
    for line in lines[1:]:
        beg, rest = line[1:].split(' ', 1)
        end, plist = rest.split(' ', 1)
        runs.append((int(beg), int(end), json.loads(plist[:-1])))
    return spans, runs


def same_where_both_fontified(a, b):
    """Whether dumps A and B give each position both fontified the same
    properties; the number of characters compared and the first difference."""
    (spans_a, runs_a), (spans_b, runs_b) = parse_dump(a), parse_dump(b)
    def at(runs):
        found = {}
        for beg, end, plist in runs:
            for pos in range(beg, end):
                found[pos] = plist
        return found
    props_a, props_b = at(runs_a), at(runs_b)
    fontified_b = {p for beg, end in spans_b for p in range(beg, end)}
    both = sorted(p for beg, end in spans_a for p in range(beg, end) if p in fontified_b)
    for pos in both:
        if props_a.get(pos) != props_b.get(pos):
            return False, len(both), (pos, props_a.get(pos), props_b.get(pos))
    return True, len(both), None


def eval_data(text):
    """The (BEG END SECONDS COVERED) a jit stamp printed."""
    parts = text.strip('()').split()
    return [int(parts[0]) if parts[0] != 'nil' else None, int(parts[1]) if parts[1] != 'nil' else None,
            float(parts[2]), parts[3] == 't']


def run(args, variant, file, latency):
    root = Path(tempfile.mkdtemp(prefix='fi-', dir=args.output))
    session = Session(args, root, latency, variant, file)
    try:
        session.start()
        return session.journey()
    finally:
        session.stop()


def summary(result, keys_typed):
    keys = result['key_ms']
    return {k: result[k] for k in ('variant', 'file', 'latency', 'instructions', 'key_samples', 'replies',
                                   'jit_calls', 'jit_covered', 'jit_max_ms', 'done_after', 'dumped',
                                   'workers', 'restarted', 'blocks_after_edit', 'keep_ms_max', 'kill_ms')} | {
        'typed_ms_median': result['typed_ms'] and statistics.median(result['typed_ms']),
        'jit_seconds': round(result['jit_seconds'], 3),
        'instructions_per_key': result['instructions'] and result['instructions'] / keys_typed,
        'key_ms_median': keys and statistics.median(keys), 'key_ms_max': keys and max(keys),
        'blocks': len(result['blocks']), 'first_block_after': result['blocks'][0][0] if result['blocks'] else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--file', action='append', type=Path)
    parser.add_argument('--keys', type=int, default=30)
    parser.add_argument('--gap', type=float, default=0.8)
    parser.add_argument('--wait', type=float, default=3.0)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--settle', type=float, default=1.0)
    parser.add_argument('--tools', default='')
    parser.add_argument('--regime', default='loadup')
    parser.add_argument('--lines', type=int, default=20)
    parser.add_argument('--variant', action='append', choices=('session', 'isolate'))
    parser.add_argument('--counted-only', action='store_true', help='only perf-counted sessions')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    results, problems = [], []
    for file in args.file or FILES:
        for latency in ((False,) if args.counted_only else (True, False)):
            for variant in args.variant or ('session', 'isolate'):
                try:
                    results.append(run(args, variant, file, latency))
                except perkey.Invalid as error:
                    problems.append(f'{variant} {file.name} latency={latency}: {error}')
    same = {}
    for file in args.file or FILES:
        for latency in (True, False):
            pair = {r['variant']: r['runs'] for r in results if r['file'] == str(file) and r['latency'] == latency}
            if len(pair) == 2:
                same[f'{file.name} latency={latency}'] = same_where_both_fontified(pair['session'], pair['isolate'])
    report = {'status': 'PASS' if not problems and same and all(v[0] for v in same.values()) else 'FAIL',
              'problems': problems, 'same_runs': same, 'sessions': [summary(r, args.keys) for r in results]}
    (args.output / 'report.json').write_text(json.dumps(report | {'raw': results}, indent=1))
    print(json.dumps(report, indent=1))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
