#!/usr/bin/env python3
"""Standing benchmark suite: fixed workloads in pinned GNU and one Romacs build.

Every workload runs in both engines on the same pinned inputs, each engine
in private HOME/XDG/TMPDIR trees, and reports per engine user-space
instructions (`perf stat -e instructions:u', the reliable metric) and wall
time (the median of --runs runs), the Romacs/GNU ratio of each, and the
change of Romacs's figures since the previous saved run.  Results go to
OUT/<commit>/<UTC time>/ as results.json and table.md; the table is printed.

Workloads (--quick runs those marked q):
  batch      GNU's and Romacs's `-Q --batch' on perfbench.el, one process per
             run: instructions are a run's less a run doing only the setup
             (perf/font-lock's method); wall is the body's own float-time.
             startup (q, to the end of command-line), font-lock-ensure on
             fileio.c, subr.el (q) and ORG-NEWS, dired of 5,000 files,
             project-find-regexp over the Emacs repository, vc-log of its
             2,000 commits from the pinned one, occur (q) and how-many (q)
             over xdisp.c.
  micro      call-path rows, instructions and ns per iteration beyond an
             empty loop: named call (q), closure call (q), mapcar of 1 and
             of 10, dolist + funcall of 10.  GNU runs them byte-compiled,
             Romacs from source, as each loads Lisp.
  ert        25 GNU ERT files, `-Q --batch -l ert -l FILE -f
             ert-run-tests-batch-and-exit' in both: totals of instructions
             and of per-file wall time, and each engine's expected count.
  gui        headless labwc, as the parity harnesses run both engines:
             first-start world build (Romacs only; GNU's dump is built with
             it), warm launch (q) spawn to Romacs's first paint and to the
             first idle moment after emacs-startup-hook (instructions to
             there); per key (q) self-insert, C-f and C-n in a 200-line text
             (whole-process instructions over the keys less the idle rate;
             wall from pre-command-hook to the next idle moment, after
             redisplay); C-x C-f over 5,000 files, fido-vertical in both
             (icomplete-compute-delay 0, so each key's completion is
             synchronous), the keys of `file-1'.
  init       the real-init profile (real_init.py, from --profile-config or
             $ROMACS_BENCH_PROFILE): init body, from the init's
             first form to its last, GNU against Romacs from source (its
             first launch, which builds the init layer) and from the layer;
             wall only.
  apps       gnosis dashboard open and back to it (gnosis_parity_live.py on
             a copy of the database, --gnosis-db or the profile's), hermes
             dashboard open (hermes_parity_live.py, read-only against the
             running dashboard, the profile's hermes_url): one run each, both engines, with --perf.

Wall times are refused (instructions still reported) for a workload during
which the 1-minute load average exceeded --max-load.  Inputs are pinned:
the Emacs tree is a --shared clone of $ROMACS_GNU_SOURCE checked out at
PIN, its files checked against SHA-256s; the 5,000-file directory is
generated with fixed contents and mtimes and checked by its manifest.

usage: perfbench.py --romacs BIN [--runtime DIR] [--commit SHA] [--quick]
                    [--runs 3] [--only GROUP|WORKLOAD ...] [--max-load 8]
                    [--out ~/.cache/romacs/perf-bench (ROMACS_PERFBENCH_OUT)]
                    [--previous RESULTS.json] [--gnosis-db COPY --gnosis-db-repo REPO]
                    [--profile-config FILE (ROMACS_BENCH_PROFILE)]
The GNU engine is $ROMACS_GNU_ORACLE (or --gnu).  A binary installed
beside its runtime (gnu-lisp/, rlisp/) uses that runtime; otherwise give
--runtime, or the binary loads the Lisp of the checkout it was built in.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import urllib.request

HERE = Path(__file__).resolve().parent
HARNESS = HERE.parent
sys.path.insert(0, str(HARNESS))
sys.path.insert(0, str(HARNESS / 'bench'))
import parity_live  # noqa: E402
import real_init  # noqa: E402

PIN = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
TREE = f'emacs-{PIN[:8]}'
PINNED = {'src/fileio.c': '0705ed5b4f93c71a45937d7de737bb54d4ae3c5cef1e565da779f4875f560c92',
          'lisp/subr.el': '55f8319dc1689044b128768c3372db6664397d61afda284bac941c687130bf32',
          'etc/ORG-NEWS': 'd12389d6044a8170912d77536363779b03509075201b0bb814d7e7874ab296b3',
          'src/xdisp.c': '1f2a3df29de9047bfa4b507528c98868cda4d964cbd13711c60ae47d4cbeb346'}
DIR_FILES, DIR_MTIME = 5000, 1767225600  # 2026-01-01T00:00:00Z
DRIVER = HERE / 'perfbench.el'
GUI_LISP = HERE / 'perfbench-gui.el'

# (row, group, batch workload, quick)
BATCH = [('startup -Q --batch', 'batch', 'startup', True),
         ('font-lock-ensure fileio.c', 'batch', 'font-lock-fileio-c', False),
         ('font-lock-ensure subr.el', 'batch', 'font-lock-subr-el', True),
         ('font-lock-ensure ORG-NEWS', 'batch', 'font-lock-org-news', False),
         ('dired 5,000 files', 'batch', 'dired-5000', False),
         ('project-find-regexp repo', 'batch', 'project-find-regexp', False),
         ('vc-log 2,000 commits', 'batch', 'vc-log-2000', False),
         ('occur xdisp.c', 'batch', 'occur-xdisp', True),
         ('how-many xdisp.c', 'batch', 'how-many-xdisp', True)]
MICRO_N = 1_000_000
# hof-inline's closure-micro file (variants.py), its loop called by the driver.
MICRO_HEAD = (';;; -*- lexical-binding: t -*-\n(require (quote cl-lib))\n(defun m-id (x) x)\n'
              '(defun m-add3 (x a b) (+ x a b))\n(defvar m-v (vector 1 2 3 4 5 6 7 8 9 10))\n'
              '(defvar m-fr (lambda (&rest xs) xs))\n(defvar m-s (list "a" "b" "c" "d" "e" "f" "g" "h" "i" "j"))\n'
              '(defun m-run (n) (let ((i 0) (a 1) (b 2) (c 3) (d 4) (s 0) (l (list 1 2 3 4 5 6 7 8 9 10))'
              ' (f0 (lambda (x) x)) (f1 nil) (f2 nil) (f4 nil)) (setq f1 (let ((a a)) (lambda (x) (+ x a))))'
              ' (setq f2 (let ((a a) (b b)) (lambda (x) (+ x a b)))) (setq f4 (let ((a a) (b b) (c c) (d d))'
              ' (lambda (x) (+ x a b c d)))) (list a b c d s l f0 f1 f2 f4) (while (< i n) BODY (setq i (1+ i))) s))\n')
# (row, loop body, quick); 'loop' is what every row's figures are beyond.
MICRO = [('loop', 'nil', True),
         ('named call', '(m-id i)', True),
         ('closure call', '(funcall f0 i)', True),
         ('mapcar 1', '(mapcar f0 (cdr (cdr (cdr (cdr (cdr (cdr (cdr (cdr (cdr l))))))))))', False),
         ('mapcar 10', '(mapcar f0 l)', False),
         ('dolist + funcall 10', '(dolist (x l) (funcall f0 x))', False)]
ERT_FILES = ['lisp/emacs-lisp/cl-lib-tests.el', 'lisp/emacs-lisp/cl-macs-tests.el', 'lisp/emacs-lisp/cl-generic-tests.el',
             'lisp/emacs-lisp/seq-tests.el', 'lisp/emacs-lisp/pcase-tests.el', 'lisp/emacs-lisp/map-tests.el',
             'lisp/emacs-lisp/generator-tests.el', 'lisp/emacs-lisp/eieio-tests/eieio-tests.el',
             'lisp/emacs-lisp/oclosure-tests.el', 'lisp/emacs-lisp/subr-x-tests.el', 'lisp/emacs-lisp/ert-tests.el',
             'lisp/emacs-lisp/cl-seq-tests.el', 'lisp/emacs-lisp/cl-extra-tests.el', 'lisp/emacs-lisp/nadvice-tests.el',
             'lisp/emacs-lisp/thunk-tests.el', 'lisp/emacs-lisp/rx-tests.el', 'lisp/emacs-lisp/macroexp-tests.el',
             'lisp/emacs-lisp/gv-tests.el', 'lisp/subr-tests.el', 'src/fns-tests.el',
             'lisp/emacs-lisp/backquote-tests.el', 'lisp/emacs-lisp/cl-print-tests.el',
             'lisp/emacs-lisp/let-alist-tests.el', 'lisp/emacs-lisp/text-property-search-tests.el',
             'lisp/emacs-lisp/ring-tests.el']
KEYS = {'self-insert': ['a'], 'C-f': ['-M', 'ctrl', 'f', '-m', 'ctrl'], 'C-n': ['-M', 'ctrl', 'n', '-m', 'ctrl']}
CTRL = lambda keys: ['-M', 'ctrl', *[a for k in keys for a in ('-k', k)], '-m', 'ctrl']  # noqa: E731
KEYSYM = {'-': 'minus'}
GROUPS = ('batch', 'micro', 'ert', 'gui', 'init', 'apps')


class Refused(Exception):
    """A workload that cannot run here, with the reason; not a number."""


# --- pure helpers -------------------------------------------------------------

def median(values):
    values = [v for v in values if v is not None]
    return statistics.median(values) if values else None


def ratio(romacs, gnu):
    return romacs / gnu if romacs is not None and gnu else None


def change(new, old):
    return (new - old) / old if new is not None and old else None


def stat_instructions(text):
    """instructions:u from `perf stat -x,' TEXT; None unless counted all the time."""
    for line in text.splitlines():
        fields = line.split(',')
        if len(fields) > 4 and fields[2].startswith('instructions') and fields[0].isdigit():
            return int(fields[0]) if fields[4] in ('100.00', '') else None
    return None


def perfbench_line(text):
    """The driver's PERFBENCH plist as a dict, or None."""
    for line in text.splitlines():
        if line.startswith('PERFBENCH ('):
            items = parity_live.read_sexp(line[len('PERFBENCH '):])
            return {str(items[i]).lstrip(':'): items[i + 1] for i in range(0, len(items) - 1, 2)}
    return None


def ert_counts(text):
    found = re.search(r'^Ran (\d+) tests?, (\d+) results? as expected', text, re.M)
    return (int(found[1]), int(found[2])) if found else None


def thread_instructions(text):
    """instructions:u of `perf stat --per-thread -x,' TEXT, less llvmpipe's
    threads; a thread that never ran (<not counted>) counts 0."""
    counts = [count(f) for f in (line.split(',') for line in text.splitlines())
              if len(f) > 5 and f[3].startswith('instructions') and not f[0].startswith('llvmpipe-')]
    return sum(counts) if counts and None not in counts else None


def count(fields):
    """One thread's count: counted all the time it ran, or 0 for a thread
    that never ran; None for a counter that was not scheduled."""
    value, running, share = fields[1], fields[4], fields[5]
    if share != '100.00':
        return None
    if value.isdigit():
        return int(value)
    return 0 if value == '<not counted>' and running == '0' else None


def first_paint(log):
    """Seconds from exec to the first paint, from --trace-latency's phases."""
    for line in log.splitlines():
        if line.startswith('romacs-startup: '):
            phase = json.loads(line[len('romacs-startup: '):])
            if phase.get('phase') == 'first-paint':
                return phase['at_ms'] / 1000
    return None


def manifest(entries):
    return hashlib.sha256(json.dumps(sorted(entries)).encode()).hexdigest()


def directory_manifest(directory):
    return manifest([(p.name, p.read_text(), int(p.stat().st_mtime)) for p in directory.iterdir()])


EXPECTED_D5000 = manifest([(f'file-{n:04}.txt', 'x\n', DIR_MTIME) for n in range(DIR_FILES)])


def row(name, group, unit, gnu, romacs, runs, note=''):
    """GNU's and Romacs's {instructions, wall, check, ...} for one workload."""
    return {'row': name, 'group': group, 'unit': unit, 'runs': runs, 'gnu': gnu, 'romacs': romacs, 'note': note}


# --- host and processes -------------------------------------------------------

def load():
    return float(Path('/proc/loadavg').read_text().split()[0])


class LoadWatch:
    """The highest 1-minute load average sampled every few seconds over a call."""

    def __init__(self, interval=2):
        self.interval, self.peak, self.done = interval, load(), threading.Event()

    def during(self, function):
        sampler = threading.Thread(target=self.sample, daemon=True)
        sampler.start()
        try:
            return function()
        finally:
            self.done.set()
            sampler.join()
            self.peak = max(self.peak, load())

    def sample(self):
        while not self.done.wait(self.interval):
            self.peak = max(self.peak, load())


def run(argv, env, cwd, timeout, perf_out=None):
    """ARGV in its own process group (killed whole on timeout), under perf
    stat when PERF_OUT is a path: (stdout+stderr text, wall, instructions)."""
    if perf_out is not None:
        argv = ['perf', 'stat', '-x,', '-e', 'instructions:u', '-o', str(perf_out), '--', *argv]
    started = time.time()
    process = subprocess.Popen(argv, env=env, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True, errors='replace', start_new_session=True)
    try:
        output, _ = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate()
        raise Refused(f'timed out after {timeout}s: {" ".join(map(str, argv[-4:]))}')
    wall = time.time() - started
    instructions = stat_instructions(Path(perf_out).read_text()) if perf_out is not None else None
    return output, wall, instructions


def private_env(base, name, extra):
    """A fresh private HOME/XDG/TMPDIR environment under BASE."""
    home = Path(tempfile.mkdtemp(prefix=f'{name}-', dir=base))
    for directory in ('.config', '.cache', '.local/share', '.local/state', 'tmp'):
        (home / directory).mkdir(parents=True)
    return {'PATH': os.environ['PATH'], 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1', 'TMPDIR': str(home / 'tmp'),
            'XDG_CONFIG_HOME': str(home / '.config'), 'XDG_CACHE_HOME': str(home / '.cache'),
            'XDG_DATA_HOME': str(home / '.local/share'), 'XDG_STATE_HOME': str(home / '.local/state'),
            **extra}


# --- fixtures -----------------------------------------------------------------

def git(*args, cwd=None):
    return subprocess.run(['git', *args], cwd=cwd, check=True, capture_output=True, text=True,
                          timeout=300).stdout


def fixtures(cache, gnu_source):
    """The fixture directory: the Emacs tree at PIN and the 5,000 files,
    made once and checked every run."""
    root = cache / 'fixtures'
    root.mkdir(parents=True, exist_ok=True)
    tree = root / TREE
    if not tree.exists():
        staging = Path(tempfile.mkdtemp(prefix='tree-', dir=root))
        git('clone', '-q', '--shared', '--no-checkout', gnu_source, str(staging / 'tree'))
        git('-c', 'advice.detachedHead=false', 'checkout', '-q', PIN, cwd=staging / 'tree')
        (staging / 'tree').rename(tree)
        staging.rmdir()
    if git('rev-parse', 'HEAD', cwd=tree).strip() != PIN:
        raise SystemExit(f'{tree} is not at {PIN}')
    if git('--no-optional-locks', 'status', '--porcelain', cwd=tree).strip():
        raise SystemExit(f'{tree} has changes; a workload wrote there (git -C {tree} status)')
    for name, digest in PINNED.items():
        if hashlib.sha256((tree / name).read_bytes()).hexdigest() != digest:
            raise SystemExit(f'{tree / name} does not match its pinned SHA-256')
    files = root / 'd5000'
    if not files.exists():
        staging = Path(tempfile.mkdtemp(prefix='d5000-', dir=root))
        for n in range(DIR_FILES):
            path = staging / f'file-{n:04}.txt'
            path.write_text('x\n')
            os.utime(path, (DIR_MTIME, DIR_MTIME))
        staging.rename(files)
    digest = directory_manifest(files)
    if digest != EXPECTED_D5000:
        raise SystemExit(f'{files} is not the 5,000-file fixture (manifest {digest})')
    return root, {'tree': str(tree), 'commit': PIN, 'pinned': PINNED, 'd5000_manifest': digest}


# --- batch workloads ----------------------------------------------------------

class Batch:
    """Both engines' `-Q --batch' on the driver, in one private tree each."""

    def __init__(self, ctx):
        self.ctx = ctx
        self.env = {name: private_env(ctx.work, f'batch-{name}', extra) for name, extra in ctx.engine_env.items()}

    def once(self, engine, workload, skip=False, extra=None):
        env = {**self.env[engine], 'PERFBENCH_WORKLOAD': workload, 'PERFBENCH_FIXTURES': str(self.ctx.fixtures),
               **(extra or {})}
        if skip:
            env['PERFBENCH_SKIP'] = '1'
        stat = self.ctx.scratch('.stat')
        argv = [self.ctx.engines[engine], '-Q', '--batch', '-l', str(DRIVER)]
        output, wall, instructions = run(argv, env, self.ctx.fixtures, 900, stat)
        stat.unlink()
        result = perfbench_line(output)
        if result is None or instructions is None:
            self.ctx.log(f'{engine} {workload}', output)
            raise Refused(f'{engine}: no result from {workload} (see logs)')
        return dict(result, instructions=instructions, process_wall=wall)

    def workload(self, name, workload):
        runs, engines = self.ctx.runs, {}
        for engine in ('gnu', 'romacs'):
            full = [self.once(engine, workload) for _ in range(runs)]
            if workload == 'startup':
                engines[engine] = {'instructions': median(r['instructions'] for r in full),
                                   'wall': median(r['process_wall'] for r in full), 'check': None}
                continue
            skipped = [self.once(engine, workload, skip=True) for _ in range(runs)]
            engines[engine] = {'instructions': median(r['instructions'] for r in full)
                               - median(r['instructions'] for r in skipped),
                               'wall': median(r['seconds'] for r in full), 'check': full[0]['check'],
                               'setup_instructions': median(r['instructions'] for r in skipped)}
        note = 'process to exit' if workload == 'startup' else 'less its setup'
        return row(name, 'batch', 'run', engines['gnu'], engines['romacs'], runs, check_note(engines, note))


def check_note(engines, note=''):
    gnu, romacs = engines['gnu'].get('check'), engines['romacs'].get('check')
    if gnu != romacs:
        note = f'{note}; result differs: GNU {gnu!r}, Romacs {romacs!r}'.lstrip('; ')
    return note


def micro_rows(ctx, batch):
    """The call-path rows: per iteration, beyond the empty loop."""
    selected = [m for m in MICRO if m[2] or not ctx.quick]
    directory = ctx.cache / 'micro'
    directory.mkdir(exist_ok=True)
    samples = {}
    for name, body, _ in selected:
        text = MICRO_HEAD.replace('BODY', body)
        source = directory / f'{hashlib.sha256(text.encode()).hexdigest()[:12]}.el'
        source.write_text(text)
        compiled = source.with_suffix('.elc')
        if not compiled.exists():
            run([ctx.engines['gnu'], '-Q', '--batch', '--eval', '(setq byte-compile-warnings nil)',
                 '-f', 'batch-byte-compile', str(source)], batch.env['gnu'], directory, 300)
        loaded = {'gnu': compiled, 'romacs': source}
        samples[name] = {engine: [batch.once(engine, 'micro', extra={'PERFBENCH_MICRO': str(loaded[engine]),
                                                                      'PERFBENCH_N': str(MICRO_N)})
                                  for _ in range(ctx.runs)] for engine in ('gnu', 'romacs')}

    def per_iteration(name, engine, key):
        return (median(s[key] for s in samples[name][engine])
                - median(s[key] for s in samples['loop'][engine])) / MICRO_N
    rows = []
    for name, body, _ in selected[1:]:
        engines = {engine: {'instructions': per_iteration(name, engine, 'instructions'),
                            'wall': per_iteration(name, engine, 'seconds')} for engine in ('gnu', 'romacs')}
        rows.append(row(f'micro {name}', 'micro', 'iteration', engines['gnu'], engines['romacs'], ctx.runs,
                        f'{body}; GNU compiled, Romacs source'))
    return rows


def ert_pass(ctx, batch, engine):
    """One pass of the 25 files in ENGINE: totals, counts and the files without a result."""
    instructions, wall, ran, expected, failed = 0, 0.0, 0, 0, []
    for name in ERT_FILES:
        path = ctx.fixtures / TREE / 'test' / name
        stat = ctx.scratch('.stat')
        output, seconds, count = run([ctx.engines[engine], '-Q', '--batch', '-l', 'ert', '-l', str(path),
                                      '-f', 'ert-run-tests-batch-and-exit'], batch.env[engine], path.parent, 900, stat)
        stat.unlink()
        counts = ert_counts(output)
        if count is None or counts is None:
            failed.append(name)
            ctx.log(f'ert {engine} {name}', output)
            continue
        instructions, wall = instructions + count, wall + seconds
        ran, expected = ran + counts[0], expected + counts[1]
    return {'instructions': instructions, 'wall': wall, 'check': f'{expected}/{ran} expected', 'failed': failed}


def ert_row(ctx, batch):
    """25 GNU ERT files, the same command in both engines: medians of
    --runs passes, withheld when any pass lacks a file's result."""
    engines = {}
    for engine in ('gnu', 'romacs'):
        passes = [ert_pass(ctx, batch, engine) for _ in range(ctx.runs)]
        failed = sorted({name for p in passes for name in p['failed']})
        totals = {k: median(p[k] for p in passes) for k in ('instructions', 'wall')}
        engines[engine] = {**({k: None for k in totals} if failed else totals),
                           'partial': totals if failed else None,
                           'check': passes[0]['check'], 'files_without_result': failed}
    note = f'GNU {engines["gnu"]["check"]}, Romacs {engines["romacs"]["check"]}; wall summed per file'
    if engines['gnu']['files_without_result'] or engines['romacs']['files_without_result']:
        note += f'; totals withheld, files without a result: GNU {engines["gnu"]["files_without_result"]},' \
                f' Romacs {engines["romacs"]["files_without_result"]}'
    return row('ERT 25 files', 'ert', 'total', engines['gnu'], engines['romacs'], ctx.runs, note)


# --- GUI workloads ------------------------------------------------------------

def gui_init(engine, idle_file=None):
    """The GUI init: GNU's --loadup settings (Romacs's own defaults off in
    Romacs), the probe and perfbench-gui.el; with IDLE_FILE, write the first
    idle moment after startup there and exit."""
    idle = ('(add-hook (quote emacs-startup-hook) (lambda () (run-with-idle-timer 0 nil'
            f' (lambda () (perfbench-gui-stamp {parity_live.lisp_string(str(idle_file))}) (kill-emacs 0)))))\n'
            if idle_file else '')
    return (';; -*- lexical-binding: t -*-\n' + (real_init.gnu_settings() if engine == 'gnu' else parity_live.ROMACS_DEFAULTS_OFF)
            + '(setq inhibit-startup-screen t)\n'
            f'(load {parity_live.lisp_string(str(GUI_LISP))} nil t)\n' + idle
            + f'(load {parity_live.lisp_string(str(parity_live.PROBE))} nil t)\n'
            '(perfbench-gui-stamp (expand-file-name "alive" parity-probe-dir))\n')


def fresh_session_dirs(root):
    """Move the last launch's compositor and probe directories aside."""
    for name in ('labwc', 'probe'):
        if (root / name).exists():
            (root / name).rename(root / f'{name}-{time.time_ns()}')
    (root / 'probe').mkdir()


def launch_rows(ctx):
    """First-start world build, then warm launches of each engine."""
    builds = []
    for _ in range(ctx.runs):
        root, home, env = parity_live.private_tree('romacs', ctx.work, ctx.tools)
        env.update(ctx.engine_env['romacs'])
        stat = root / 'world.stat'
        output, wall, instructions = run([ctx.engines['romacs'], '--loadup', '--world-build'], env, root / 'work', 900, stat)
        if instructions is None or 'romacs-world:' not in output:
            ctx.log('world-build', output)
        builds.append({'instructions': instructions, 'wall': wall})
    world = row('first-start world build', 'gui', 'run', None,
                {'instructions': median(b['instructions'] for b in builds), 'wall': median(b['wall'] for b in builds)},
                ctx.runs, 'Romacs only: --loadup --world-build in a fresh store')
    launches = {}
    for engine in ('gnu', 'romacs'):
        root, home, env = parity_live.private_tree(engine, ctx.work, ctx.tools)
        env.update(ctx.engine_env[engine])
        idle_file = root / 'idle'
        init = gui_init(engine, idle_file)
        if engine == 'gnu':
            (root / 'init.el').write_text(init)
            argv = [ctx.engines['gnu'], '-Q', '--font', 'Iosevka-14', '-l', str(root / 'init.el')]
        else:
            (home / '.config/romacs').mkdir()
            (home / '.config/romacs/init.el').write_text(init)
            env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
            parity_live.prepare_world(ctx.engines['romacs'], env, root / 'work')
            argv = [ctx.engines['romacs'], '--loadup', '--trace-latency']
        samples = []
        for n in range(ctx.runs + 1):  # the first launch warms the page cache
            fresh_session_dirs(root)
            if idle_file.exists():
                idle_file.unlink()
            stat = root / f'launch-{n}.stat'
            launcher = parity_live.Engine(engine, ['perf', 'stat', '-x,', '-e', 'instructions:u', '-o', str(stat),
                                                   '--', *argv], root, None, dict(env), 120)
            try:
                launcher.start(ctx.tools)
                # Engine.start times from the editor's spawn, after its compositor.
                spawned = float((root / 'probe/alive').read_text()) - launcher.startup_seconds
                parity_live.wait(idle_file.exists, 120, f'{engine} idle', launcher.process)
                launcher.process.wait(timeout=30)
            finally:
                launcher.stop()
            paint = first_paint((root / 'engine.log').read_text(errors='replace'))
            samples.append({'idle': float(idle_file.read_text()) - spawned,
                            'paint': paint,
                            'instructions': stat_instructions(stat.read_text())})
        launches[engine] = samples[1:]
    idle = row('warm launch to idle', 'gui', 'run',
               *[{'instructions': median(s['instructions'] for s in launches[e]),
                  'wall': median(s['idle'] for s in launches[e])} for e in ('gnu', 'romacs')],
               ctx.runs, 'spawn to the first idle after emacs-startup-hook; instructions to exit there')
    paint = row('warm launch to first paint', 'gui', 'run', None,
                {'instructions': None, 'wall': median(s['paint'] for s in launches['romacs'])}, ctx.runs,
                'Romacs only (its --trace-latency first-paint); GNU has no paint mark')
    return [world, paint, idle]


class Session:
    """One engine's GUI session on the probe, measuring key phases."""

    def __init__(self, ctx, engine):
        root, home, env = parity_live.private_tree(engine, ctx.work, ctx.tools)
        env.update(ctx.engine_env[engine])
        init = gui_init(engine)
        start = parity_live.gnu_engine if engine == 'gnu' else \
            lambda *a: parity_live.romacs_engine(ctx.engines['romacs'], *a)
        self.engine = start(root, home, env, init, None, 120, ctx.tools)
        self.root = root

    def ask(self, form, timeout=60):
        return self.engine.ask(form, timeout)

    def counted(self, act, seconds):
        """Instructions and seconds over ACT, which takes about SECONDS, the
        probe paused: every thread but llvmpipe's, the software GL that
        stands in for the GPU on the headless compositor."""
        self.ask(f'(perfbench-gui-pause {seconds + 1.5})')
        started = time.time()
        counter = subprocess.Popen(['perf', 'stat', '--per-thread', '-x,', '-e', 'instructions:u',
                                    '-p', str(self.engine.process.pid)],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        time.sleep(.25)
        act()
        time.sleep(.25)
        counter.send_signal(signal.SIGINT)
        elapsed = time.time() - started
        _, err = counter.communicate(timeout=10)
        return thread_instructions(err), elapsed

    def idle_rate(self, seconds=3):
        instructions, elapsed = self.counted(lambda: time.sleep(seconds), seconds)
        if instructions is None:
            raise Refused('idle counter not scheduled the whole time')
        return instructions / elapsed

    def phase(self, groups, gap, rate):
        """Instructions and wall per key over GROUPS, one key each."""
        self.ask('(perfbench-gui-reset)')

        def type_keys():
            for group in groups:
                self.engine.keys(group)
                time.sleep(gap)
        instructions, seconds = self.counted(type_keys, len(groups) * (gap + .1))
        latencies = self.ask('perfbench-gui-latencies') or []
        return {'instructions': (instructions - rate * seconds) / len(groups) if instructions is not None else None,
                'wall': median(latencies), 'keys': len(groups), 'samples': len(latencies)}

    def stop(self):
        self.engine.stop()
        shutil.rmtree(self.root, ignore_errors=True)


def key_rows(ctx, keys=40, gap=.2):
    """Per-key self-insert, C-f, C-n; then C-x C-f over 5,000 files."""
    rounds = ctx.runs
    text = ''.join(f'line {i} of some ordinary prose text, about sixty bytes long\n' for i in range(200))
    results = {}
    for engine in ('gnu', 'romacs'):
        session = Session(ctx, engine)
        try:
            path = session.root / 'work/text.txt'
            path.write_text(text)
            session.ask(f'(progn (find-file {parity_live.lisp_string(str(path))}) (setq auto-save-timeout nil)'
                        ' (blink-cursor-mode -1) (goto-char (point-min)) t)')
            time.sleep(2)
            rate = session.idle_rate()
            phases = {name: [] for name in KEYS}
            for _ in range(rounds):
                for name, group in KEYS.items():
                    phases[name].append(session.phase([group] * keys, gap, rate))
            results[engine] = {name: {'instructions': median(p['instructions'] for p in runs),
                                      'wall': median(p['wall'] for p in runs),
                                      'samples': sum(p['samples'] for p in runs)}
                               for name, runs in phases.items()}
            if not ctx.quick:
                results[engine]['C-x C-f'] = completion(session, ctx.fixtures / 'd5000', rate, rounds)
        finally:
            session.stop()
    rows = [row(f'key {name}', 'gui', 'key', results['gnu'][name], results['romacs'][name], rounds,
                f'{keys} keys x {rounds}; wall pre-command to idle; samples GNU {results["gnu"][name]["samples"]},'
                f' Romacs {results["romacs"][name]["samples"]}')
            for name in KEYS]
    if not ctx.quick:
        rows.append(row('C-x C-f 5,000 files, per key', 'gui', 'key', results['gnu']['C-x C-f'],
                        results['romacs']['C-x C-f'], rounds,
                        check_note({e: results[e]['C-x C-f'] for e in results},
                                   'fido-vertical, keys of file-1; candidates GNU'
                                   f' {results["gnu"]["C-x C-f"]["candidates"]},'
                                   f' Romacs {results["romacs"]["C-x C-f"]["candidates"]}')))
    return rows


def completion(session, directory, rate, rounds):
    session.ask('(progn (setq icomplete-compute-delay 0) (fido-vertical-mode 1)'
                f' (with-current-buffer (window-buffer (selected-window)) (setq default-directory'
                f' {parity_live.lisp_string(str(directory) + "/")})) t)')
    runs, check, candidates = [], None, None
    for _ in range(rounds):
        session.engine.keys(CTRL(['x', 'f']))
        parity_live.wait(lambda: session.ask('(minibufferp (window-buffer (selected-window)))'), 30,
                         'C-x C-f prompt', session.engine.process)
        time.sleep(1)
        runs.append(session.phase([['-k', KEYSYM[c]] if c in KEYSYM else [c] for c in 'file-1'], .3, rate))
        check = session.ask('(file-name-nondirectory (minibuffer-contents-no-properties))')
        candidates = session.ask('(safe-length (completion-all-sorted-completions))')
        session.engine.keys(CTRL(['g']))
        time.sleep(.5)
    return {'instructions': median(r['instructions'] for r in runs), 'wall': median(r['wall'] for r in runs),
            'check': check, 'candidates': candidates, 'samples': sum(r['samples'] for r in runs)}


# --- real init -----------------------------------------------------------------

def init_rows(ctx):
    """The real init's body: GNU, Romacs from source (layer build), Romacs's layer."""
    config = real_init.load(ctx.args.profile_config)
    real_init.profile_init(config)
    base = ctx.cache / 'init'
    base.mkdir(exist_ok=True)
    project = real_init.default_project(config)
    bodies, unlayered = {}, []
    for engine in ('gnu', 'romacs'):
        root, home, env = parity_live.private_tree(engine, ctx.work, ctx.tools)
        env.update(ctx.engine_env[engine])
        (root / 'mcp').mkdir(mode=0o700)
        real_init.populate(home, project, None, None, config)
        lisp = real_init.lisp_dir(base, engine, config)
        source = real_init.init_source(engine, lisp, root / 'mcp/port', secrets.token_hex(24), 'init', config)
        first, rest = source.split('\n', 1)
        init = (f'{first}\n(with-temp-file (expand-file-name "init-start" (getenv "PARITY_PROBE_DIR"))'
                f' (insert (number-to-string (float-time))))\n{rest}')

        def body(engine_):
            probe = root / 'probe'
            return float((probe / 'alive').read_text()) - float((probe / 'init-start').read_text()), \
                engine_.startup_seconds
        samples = []
        for n in range(ctx.runs + (1 if engine == 'romacs' else 0)):
            fresh_session_dirs(root)
            if n == 0:
                start = parity_live.gnu_engine if engine == 'gnu' else \
                    lambda *a: parity_live.romacs_engine(ctx.engines['romacs'], *a)
                launched = start(root, home, env, init, lisp, 120, ctx.tools)
            else:
                argv = [ctx.engines['romacs'], '--loadup'] if engine == 'romacs' else [ctx.engines['gnu'], '--font', 'Iosevka-14']
                launched = parity_live.Engine(engine, argv, root, lisp, env, 120).start(ctx.tools)
            try:
                samples.append(body(launched))
                if engine == 'romacs' and n == 0:
                    layer_built(root)
            finally:
                launched.stop()
            if engine == 'romacs':
                log = (root / 'engine.log').read_text(errors='replace')
                ctx.log(f'init romacs launch {n}', log)
                if n > 0:
                    unlayered.append(layer_refusal(log))
                elif 'build failed' in log:
                    unlayered.append('layer build failed: ' + log[log.index('build failed'):][:100])
            if engine == 'romacs' and n == 0:
                run([ctx.engines['romacs'], '--loadup', '--world-build'], env, root / 'work', 900)
        bodies[engine] = samples
    gnu = {'instructions': None, 'wall': median(s[0] for s in bodies['gnu'])}
    rows = [row('init body, Romacs source', 'init', 'run', gnu,
                {'instructions': None, 'wall': bodies['romacs'][0][0]}, 1,
                'first launch: from source, builds the init layer; wall only'),
            row('init body, Romacs layer', 'init', 'run', gnu,
                {'instructions': None, 'wall': median(s[0] for s in bodies['romacs'][1:])}, ctx.runs,
                layer_note(unlayered)),
            row('start to init done, layer', 'init', 'run',
                {'instructions': None, 'wall': median(s[1] for s in bodies['gnu'])},
                {'instructions': None, 'wall': median(s[1] for s in bodies['romacs'][1:])}, ctx.runs,
                'spawn to the init\'s last form; wall only')]
    return rows


def layer_refusal(log):
    """Why a launch booted from the base world, not the init layer; None if it restored the layer."""
    for line in log.splitlines():
        if line.startswith('romacs-world: '):
            event = json.loads(line[len('romacs-world: '):])
            if event.get('mode') == 'restored' and 'layer' in event:
                return None
            if event.get('event') == 'layer' and event.get('boot') == 'base':
                return json.dumps(event.get('reason'))[:120]
    return 'no world line'


def layer_note(refusals):
    if not any(refusals):
        return 'init layer restored; wall only'
    return (f'NOT the layer: launches booted the base world'
            f' ({"; ".join(r for r in refusals if r)[:240]}); see logs/init-romacs-launch-*; wall only')


def layer_built(root, timeout=300):
    """Wait for launch 1's background layer build to end."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        log = (root / 'engine.log').read_text(errors='replace')
        if '"layer-build"' in log and any(w in log for w in ('"layer-built"', 'build failed', 'refused')):
            return
        time.sleep(1)


# --- gnosis and hermes ---------------------------------------------------------

def app_rows(ctx):
    rows = []
    config = real_init.load(ctx.args.profile_config)
    db = ctx.args.gnosis_db or config.get('gnosis_db')
    repo = ctx.args.gnosis_db_repo or config.get('gnosis_db_repo')
    if db and repo:
        steps = parity_steps(ctx, 'gnosis', ['--db', str(db), '--db-repo', str(repo),
                                             '--workflow', 'dashboard', '--until', 'back'])
        rows += [row('gnosis dashboard open', 'apps', 'run', *steps['open'], 1, 'gnosis_parity_live, DB copy'),
                 row('gnosis back to dashboard', 'apps', 'run', *steps['back'], 1, 'from all themata, q')]
    else:
        ctx.skipped.append('gnosis: no --gnosis-db/--gnosis-db-repo')
    try:
        urllib.request.urlopen(real_init.hermes_url(config) + '/', timeout=3)
    except OSError as error:
        ctx.skipped.append(f'hermes: no dashboard at {real_init.hermes_url(config)} ({error})')
        return rows
    steps = parity_steps(ctx, 'hermes', ['--workflow', 'sessions', '--until', 'open'])
    rows.append(row('hermes dashboard open', 'apps', 'run', *steps['open'], 1, 'hermes_parity_live, read-only'))
    return rows


def profile_args(ctx):
    path = real_init.config_path(ctx.args.profile_config)
    return ['--profile-config', str(path.resolve())] if path else []


def parity_steps(ctx, harness, args):
    output = Path(tempfile.mkdtemp(prefix=f'{harness}-', dir=ctx.work))
    (ctx.cache / harness).mkdir(exist_ok=True)
    log, _, _ = run([sys.executable, '-B', str(HARNESS / f'{harness}_parity_live.py'), '--romacs', ctx.engines['romacs'],
                     '--gnu', ctx.engines['gnu'], '--output', str(output / 'out'), '--work', str(ctx.cache / harness),
                     '--perf', *profile_args(ctx), *args], {**os.environ, 'TMPDIR': str(output)}, HARNESS, 1800)
    ctx.log(f'{harness} harness', log)
    report = json.loads((output / 'out/report.json').read_text())
    steps = {}
    for workflow in report['workflows']:
        for step in workflow['steps']:
            timing = step.get('timing') or {}
            steps[step['step']] = [{'instructions': timing.get(e, {}).get('instructions'),
                                    'wall': timing.get(e, {}).get('seconds')} for e in ('gnu', 'romacs')]
        if workflow.get('error'):
            raise Refused(f'{harness} {workflow["workflow"]}: {workflow["error"]}')
    return steps


# --- results ----------------------------------------------------------------------

def gate_walls(rows, max_load):
    """Clear the wall times of rows measured above MAX_LOAD."""
    for r in rows:
        if r.get('load', 0) > max_load:
            for engine in ('gnu', 'romacs'):
                if r[engine] and r[engine].get('wall') is not None:
                    r[engine]['wall_refused'] = r[engine].pop('wall')
            r['note'] = f'wall refused: load {r["load"]:.1f} > {max_load}; {r["note"]}'
    return rows


def with_ratios(rows, previous):
    """ROWS with Romacs/GNU ratios and changes against PREVIOUS's rows."""
    before = {r['row']: r for r in (previous or {}).get('rows', [])}
    for r in rows:
        for metric in ('instructions', 'wall'):
            value = (r['romacs'] or {}).get(metric)
            r[f'{metric}_ratio'] = ratio(value, (r['gnu'] or {}).get(metric))
            r[f'{metric}_change'] = change(value, ((before.get(r['row']) or {}).get('romacs') or {}).get(metric))
    return rows


def scaled(value, unit):
    if value is None:
        return '-'
    if unit == 'instructions':
        for limit, suffix in ((1e9, 'G'), (1e6, 'M'), (1e3, 'k')):
            if abs(value) >= limit:
                return f'{value / limit:.3g}{suffix}'
        return f'{value:.0f}'
    for limit, suffix in ((1, 's'), (1e-3, 'ms'), (1e-6, 'us')):
        if abs(value) >= limit:
            return f'{value / limit:.3g} {suffix}'
    return f'{value * 1e9:.3g} ns'


def times(value):
    return '-' if value is None else f'{value:.2f}x'


def percent(value):
    return '-' if value is None else f'{value * 100:+.1f}%'


def table(results):
    """The markdown table of RESULTS."""
    meta = results['meta']
    lines = [f'# perfbench {meta["commit"][:12]} ({meta["started"]})', '',
             f'Romacs `{meta["romacs"]}` (sha256 {meta["romacs_sha256"][:12]}), runtime `{meta["runtime"]}`; '
             f'GNU `{meta["gnu"]}`.  Runs {meta["runs"]}{", quick" if meta["quick"] else ""}; '
             f'load {meta["load_start"]:.1f} at start, {meta["load_end"]:.1f} at end, walls refused above '
             f'{meta["max_load"]}.  Previous: {meta["previous"] or "none"}.', '',
             '| workload | per | GNU instr | Romacs instr | R/G | Δ instr | GNU wall | Romacs wall | R/G | Δ wall | note |',
             '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|']
    for r in results['rows']:
        gnu, romacs = r['gnu'] or {}, r['romacs'] or {}
        lines.append(f'| {r["row"]} | {r["unit"]} | {scaled(gnu.get("instructions"), "instructions")} | '
                     f'{scaled(romacs.get("instructions"), "instructions")} | {times(r["instructions_ratio"])} | '
                     f'{percent(r["instructions_change"])} | {scaled(gnu.get("wall"), "s")} | '
                     f'{scaled(romacs.get("wall"), "s")} | {times(r["wall_ratio"])} | {percent(r["wall_change"])} | '
                     f'{r["note"]} |')
    for line in results['skipped']:
        lines.append(f'\nNot run: {line}')
    return '\n'.join(lines) + '\n'


def previous_results(out_root, explicit, current):
    if explicit:
        return Path(explicit)
    found = sorted((p for p in out_root.glob('*/*/results.json') if p.parent != current),
                   key=lambda p: json.loads(p.read_text())['meta']['started'])
    return found[-1] if found else None


# --- main -----------------------------------------------------------------------

class Context:
    def __init__(self, args, run_dir, cache, fixtures_dir, runtime):
        self.args, self.run_dir, self.cache, self.fixtures = args, run_dir, cache, fixtures_dir
        self.runs, self.quick, self.tools = args.runs, args.quick, args.tools
        self.engines = {'gnu': args.gnu, 'romacs': str(Path(args.romacs).resolve())}
        self.engine_env = {'gnu': {}, 'romacs': {'ROMACS_RUNTIME_ROOT': str(runtime),
                                                 'ROMACS_LISP_DIR': str(runtime / 'gnu-lisp'),
                                                 'ROMACS_GNU_SOURCE': os.environ.get('ROMACS_GNU_SOURCE', '')}}
        # Private trees hold compositor sockets, which take 108 bytes at most.
        (cache.parent / 'w').mkdir(exist_ok=True)
        self.work = Path(tempfile.mkdtemp(dir=cache.parent / 'w'))
        if len(str(self.work)) > 75:  # + /romacs-XXXXXXXX/wayland-NN
            raise SystemExit(f'{self.work}: too long for socket paths; give a shorter --out')
        self.skipped = []

    def scratch(self, suffix):
        """A fresh path in the work directory."""
        return self.work / f'{time.monotonic_ns()}{suffix}'

    def log(self, name, text):
        logs = self.run_dir / 'logs'
        logs.mkdir(exist_ok=True)
        (logs / (re.sub(r'[^A-Za-z0-9_.-]', '-', name) + '.log')).write_text(text)


def check_selectors(only, plan):
    known = {name for name, *_ in plan} | set(GROUPS)
    unknown = sorted(set(only or ()) - known)
    if unknown:
        raise SystemExit(f'--only: unknown {unknown}; choose from {sorted(known)}')


def selected(args, name, group, quick):
    if args.only:
        return name in args.only or group in args.only
    return quick or not args.quick


def runtime_of(args):
    binary = Path(args.romacs).resolve()
    for candidate in (args.runtime, os.environ.get('ROMACS_RUNTIME_ROOT'), binary.parent):
        if candidate and (Path(candidate) / 'gnu-lisp').is_dir() and (Path(candidate) / 'rlisp').is_dir():
            return Path(candidate).resolve()
    raise SystemExit('no runtime: give --runtime DIR (holding gnu-lisp/ and rlisp/)')


def commit_of(args):
    if args.commit:
        return args.commit
    build = Path(args.romacs).resolve().parent / 'build.json'
    if build.exists():
        return json.loads(build.read_text())['commit']
    raise SystemExit('no build.json beside the binary: give --commit SHA')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--runtime')
    parser.add_argument('--commit')
    parser.add_argument('--quick', action='store_true')
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--only', nargs='+', help='groups, or workloads (a batch row, micro, ERT 25 files, launch,'
                        ' keys, init, apps)')
    parser.add_argument('--max-load', type=float, default=8.0)
    parser.add_argument('--out', default=os.environ.get('ROMACS_PERFBENCH_OUT', '~/.cache/romacs/perf-bench'))
    parser.add_argument('--previous', help='results.json to compare with (default: the newest saved run)')
    parser.add_argument('--gnosis-db')
    parser.add_argument('--gnosis-db-repo')
    parser.add_argument('--profile-config', default=os.environ.get(real_init.CONFIG_ENV),
                        help='the real-init profile config (real_init.py)')
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--keep', action='store_true', help='keep the private trees')
    args = parser.parse_args()
    if not args.gnu or not os.environ.get('ROMACS_GNU_SOURCE'):
        raise SystemExit('needs ROMACS_GNU_ORACLE (or --gnu) and ROMACS_GNU_SOURCE')
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    runtime, commit = runtime_of(args), commit_of(args)
    os.environ.update(ROMACS_RUNTIME_ROOT=str(runtime), ROMACS_LISP_DIR=str(runtime / 'gnu-lisp'))
    parity_live.GNU = args.gnu
    out_root = Path(args.out).expanduser()
    cache = out_root / 'cache'
    cache.mkdir(parents=True, exist_ok=True)
    started = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())
    run_dir = out_root / commit[:12] / started
    run_dir.mkdir(parents=True)
    fixtures_dir, pins = fixtures(cache, os.environ['ROMACS_GNU_SOURCE'])
    ctx = Context(args, run_dir, cache, fixtures_dir, runtime)
    meta = {'commit': commit, 'started': started, 'romacs': ctx.engines['romacs'],
            'romacs_sha256': hashlib.sha256(Path(args.romacs).read_bytes()).hexdigest(), 'runtime': str(runtime),
            'gnu': args.gnu, 'runs': args.runs, 'quick': args.quick, 'max_load': args.max_load,
            'load_start': load(), 'fixtures': pins, 'cpus': os.cpu_count()}
    print(f'perfbench {commit[:12]} -> {run_dir}; load {meta["load_start"]:.1f}', flush=True)
    rows = []
    try:
        batch = Batch(ctx)
        plan = [(name, group, quick, lambda w=workload, n=name: [batch.workload(n, w)])
                for name, group, workload, quick in BATCH]
        plan += [('micro', 'micro', True, lambda: micro_rows(ctx, batch)),
                 ('ERT 25 files', 'ert', False, lambda: [ert_row(ctx, batch)]),
                 ('launch', 'gui', True, lambda: launch_rows(ctx)),
                 ('keys', 'gui', True, lambda: key_rows(ctx)),
                 ('init', 'init', False, lambda: init_rows(ctx)),
                 ('apps', 'apps', False, lambda: app_rows(ctx))]
        check_selectors(args.only, plan)
        for name, group, quick, function in plan:
            if not selected(args, name, group, quick):
                continue
            started_at, watch = time.time(), LoadWatch()
            try:
                made = watch.during(function)
            except Exception as error:  # noqa: BLE001 -- one workload's failure is reported, the rest still run
                ctx.skipped.append(f'{name}: {error}')
                ctx.log(f'{name} error', traceback.format_exc())
                print(f'  {name}: not run: {error}', flush=True)
                continue
            for r in made:
                r['load'] = watch.peak
            rows += made
            print(f'  {name}: {time.time() - started_at:.0f}s, load up to {watch.peak:.1f}', flush=True)
    finally:
        if not args.keep:
            shutil.rmtree(ctx.work, ignore_errors=True)
    meta['load_end'] = load()
    previous = previous_results(out_root, args.previous, run_dir)
    meta['previous'] = str(previous) if previous else None
    results = {'meta': meta, 'skipped': ctx.skipped,
               'rows': with_ratios(gate_walls(rows, args.max_load), json.loads(previous.read_text()) if previous else None)}
    (run_dir / 'results.json').write_text(json.dumps(results, indent=1) + '\n')
    text = table(results)
    (run_dir / 'table.md').write_text(text)
    print(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
