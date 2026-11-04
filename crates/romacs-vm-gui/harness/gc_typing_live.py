#!/usr/bin/env python3
"""Typing while background work allocates: key latency against collections.

One fresh private headless labwc session runs `romacs-vm-gui -Q --loadup
FILE` (perkey_live's session).  Its M-: setup turns the idle auto-save
off, loads org, eshell, dired and cc-mode, so the heap is a working
session's, and adds a `post-command-hook' recording, for every command,
its event, `gcs-done', `gc-elapsed' and `float-time'.  Once the evaluator
is quiet, a second M-: starts a timer that allocates the way an agent's
stream does: every --tick seconds, events of a plist and two strings, the
text inserted into a buffer that is emptied past 200 KB, --rate heap units
a second.  After --warm seconds of that it types --keys `Q' keys in bursts
of 4 to 20 keys 120-220 ms apart, with a pause between bursts drawn from
PAUSES (a fixed schedule from --seed): into the visited file, or with
--prompt into a `read-string' prompt opened for them.  Then it stops the
stream and reads the hook's records back.

A key's latency is from just before it was typed to the end of its
command, both by the wall clock: the time it waited behind anything the
evaluator was doing, a collection included, and its command's.  Whether a
key met a collection is an estimate: a collection is placed as ending
with the command after which `gcs-done' first shows it, lasting its
`gc-elapsed' (an idle collection may have ended earlier in the pause), and
a key met it when its own interval overlaps that and its latency was at
least MET seconds, so a collection finished before the key arrived, which
leaves the key fast, does not count.

Reports keys, latency p50/p99/max, the collections during typing (count,
total and worst seconds), the keys that met one with their worst latency,
and the worst latency of the keys that met none.  Refuses, rather than
reports, a run whose typed keys and recorded commands do not match.

usage: gc_typing_live.py --binary PATH --output DIR [--keys 400] [--seed 1]
                         [--rate 300000] [--tick 0.02] [--warm 15] [--prompt]
                         [--tools DIR:DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import tempfile
import time

import perkey_live as perkey

# Seconds between bursts, and their weights: mostly short pauses, a tail
# of thinking.
PAUSES = ((0.3, 6), (0.6, 6), (1.0, 5), (1.5, 4), (2.2, 3), (3.0, 2), (4.5, 2), (6.5, 1), (9.0, 1))
# Heap units (objects, and payload in 16-byte units) one stream event
# allocates, roughly: a 6-cons plist, a vector and two short strings.
EVENT_UNITS = 16
# The key typed: no setup form holds it, so its commands are told apart.
KEY = 'Q'
HOOK = ('(progn (defvar gc-typing--records nil)'
        ' (add-hook (quote post-command-hook) (lambda () (push (list this-command last-command-event'
        ' gcs-done gc-elapsed (float-time)) gc-typing--records))))')
STREAM = ('(progn (defvar gc-typing--n 0)'
          ' (defun gc-typing--stream () (with-current-buffer (get-buffer-create " *gc-typing-stream*")'
          ' (dotimes (_ {events}) (setq gc-typing--n (1+ gc-typing--n))'
          ' (let ((s (format "event %d %s" gc-typing--n (make-string 40 ?x))))'
          ' (insert s) (list :type (quote delta) :text s :n (vector gc-typing--n))))'
          ' (when (> (buffer-size) 200000) (erase-buffer))))'
          ' (setq gc-typing--timer (run-with-timer {tick} {tick} (function gc-typing--stream))))')
DUMP = ('(progn (cancel-timer gc-typing--timer) (with-temp-file "{path}"'
        ' (dolist (r (reverse gc-typing--records)) (prin1 r (current-buffer)) (terpri (current-buffer)))))')


def schedule(keys, seed):
    """The gap after each of KEYS keys: bursts with pauses between them."""
    rng = random.Random(seed)
    gaps = []
    while len(gaps) < keys:
        burst = rng.randint(4, 20)
        gaps += [rng.uniform(.12, .22) for _ in range(burst - 1)]
        gaps.append(rng.choices([p for p, _ in PAUSES], [w for _, w in PAUSES])[0])
    return gaps[:keys]


def read_records(path):
    """The hook's records, one Lisp list per line: (command event gcs elapsed time)."""
    rows = []
    for line in path.read_text().splitlines():
        fields = line.strip('()').split()
        if len(fields) == 5:
            rows.append((fields[0], fields[1], int(fields[2]), float(fields[3]), float(fields[4])))
    return rows


# Seconds of latency from which a key overlapping a collection met it.
MET = .05


def attribute(typed_at, records):
    """Per typed key: (latency s, collections, their seconds, met).
    TYPED_AT holds the wall times the keys were typed; RECORDS their
    commands' records, the one before the first key's first.  A key's
    collections are those `gcs-done' first shows after its command."""
    spans = []
    for (_, _, g0, e0, _), (_, _, g1, e1, done) in zip(records, records[1:]):
        if g1 > g0:
            spans.append((done - (e1 - e0), done))
    rows = []
    for i, typed in enumerate(typed_at):
        (_, _, g0, e0, _), (_, _, g1, e1, done) = records[i], records[i + 1]
        latency = done - typed
        met = latency >= MET and any(start < done and typed < end for start, end in spans)
        rows.append((latency, g1 - g0, e1 - e0, met))
    return rows


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(fraction * len(ordered)))]


def summary(rows):
    latencies = [r[0] for r in rows]
    met = [r for r in rows if r[3]]
    # The first key's interval also holds the warm-up's collections.
    collections = [(r[1], r[2]) for r in rows[1:] if r[1]]
    return {'keys': len(rows), 'latency_ms': {'p50': percentile(latencies, .5) * 1e3,
                                              'p99': percentile(latencies, .99) * 1e3, 'max': max(latencies) * 1e3},
            'collections': {'count': sum(n for n, _ in collections), 'seconds': sum(s for _, s in collections),
                            'worst_interval_s': max((s for _, s in collections), default=0)},
            'keys_met_collection': len(met), 'met_worst_ms': max((r[0] for r in met), default=0) * 1e3,
            'met_latencies_ms': [round(r[0] * 1e3, 1) for r in met],
            'clear_worst_ms': max((r[0] for r in rows if not r[3]), default=0) * 1e3,
            'keys_over_100ms': sum(1 for r in rows if r[0] > .1)}


class Session(perkey.Session):
    def setup_form(self):
        loads = ' '.join(f'(require (quote {f}))' for f in ('org', 'eshell', 'dired', 'cc-mode'))
        return f"(progn {super().setup_form()} {loads} {HOOK} {self.args.setup or ''})"

    def eval_form(self, form, wait=True):
        """FORM through M-:, waiting for its reply unless not WAIT; the
        stdout records before it."""
        before = len(perkey.records(self.stdout))
        self.type(['-M', 'alt', ':', '-m', 'alt'])
        self.type([form])
        self.type(['-k', 'Return'])
        if wait:
            self.reply(before, form)
        return before

    def reply(self, before, form):
        done = lambda: [r for r in perkey.records(self.stdout)[before:] if r.get('native_command') == 'eval-expression']
        self.wait(done, self.args.timeout, f'M-: {form[:40]}')
        if any(r.get('signal') or r.get('native_error') for r in done()):
            raise perkey.Invalid(f'M-: {form[:60]} signalled: {done()}')

    def measure(self):
        args = self.args
        events = max(1, int(args.rate * args.tick / EVENT_UNITS))
        self.eval_form(STREAM.format(events=events, tick=args.tick))
        time.sleep(args.warm)
        if args.prompt:
            self.eval_form('(read-string "gc-typing: ")', wait=False)
            time.sleep(1)
        gaps = schedule(args.keys, args.seed)
        typed_at = []
        for gap in gaps:
            typed_at.append(time.time())
            self.type([KEY])
            time.sleep(gap)
        if args.prompt:
            # The Return ends the read and its M-:; the host replies with the
            # key's own command.
            before = len(perkey.records(self.stdout))
            self.type(['-k', 'Return'])
            self.wait(lambda: any('native_command' in r for r in perkey.records(self.stdout)[before:]),
                      args.timeout, 'the prompt\'s Return')
        path = self.root / 'records.el'
        self.eval_form(DUMP.format(path=path))
        self.wait(path.exists, 10, 'records.el')
        records = read_records(path)
        shutil.copy(path, args.output / 'records.el')
        ours = [i for i, r in enumerate(records) if r[0] == 'self-insert-command' and r[1] == str(ord(KEY))]
        problems = []
        if len(ours) != args.keys or ours != list(range(ours[0], ours[0] + args.keys)) or ours[0] == 0:
            problems.append(f'{len(ours)} records of typed keys for {args.keys} keys, contiguous: '
                            f'{bool(ours) and ours == list(range(ours[0], ours[0] + len(ours)))}')
            rows = []
        else:
            rows = attribute(typed_at, records[ours[0] - 1:ours[-1] + 1])
        return {'typing_s': typed_at[-1] - typed_at[0], 'events_per_tick': events, 'problems': problems,
                'typed_at': typed_at, 'rows': rows, 'summary': summary(rows) if rows else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--keys', type=int, default=400)
    parser.add_argument('--seed', type=int, default=1)
    parser.add_argument('--rate', type=int, default=300_000, help='heap units the stream allocates a second')
    parser.add_argument('--tick', type=float, default=.02, help="seconds between the stream's events")
    parser.add_argument('--warm', type=float, default=15, help='seconds the stream runs before typing')
    parser.add_argument('--prompt', action='store_true', help='type into a read-string prompt')
    parser.add_argument('--setup', help='Lisp the setup also evaluates, such as a variant\'s settings')
    parser.add_argument('--settle', type=float, default=3)
    parser.add_argument('--timeout', type=float, default=240)
    parser.add_argument('--tools', help='directories with labwc, wlr-randr and wtype, PATH-style')
    parser.add_argument('--mode', default='1600x1000')
    args = parser.parse_args()
    # perkey_live's session options this journey fixes.
    args.regime, args.lines, args.sample, args.state, args.gap = 'loadup', 200, None, False, .05
    args.classic, args.line_numbers, args.populate, args.split = False, None, False, 1
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.output.mkdir(parents=True)
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    with tempfile.TemporaryDirectory(prefix='gctyping-', dir=runtime) as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        session = Session(args, root, False)
        try:
            session.start()
            result = session.measure()
        except (perkey.Invalid, AssertionError, subprocess.SubprocessError) as error:
            result = {'problems': [repr(error)], 'summary': None}
        finally:
            session.stop()
            for name in ('binary.stdout', 'binary.stderr', 'compositor.log'):
                if (root / name).exists():
                    shutil.copy(root / name, args.output / name)
    report = {'binary': str(args.binary), 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'keys': args.keys, 'seed': args.seed, 'rate': args.rate, 'tick': args.tick, 'warm_s': args.warm,
              'prompt': args.prompt, 'setup': args.setup, **result}
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({k: report.get(k) for k in ('summary', 'problems', 'typing_s', 'events_per_tick')}, indent=1))
    return 0 if report.get('summary') and not report['problems'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
