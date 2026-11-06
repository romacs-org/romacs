#!/usr/bin/env python3
"""Standing benchmark suite: fixed workloads in GNU Emacs and one Romacs build.

The GNU engine is this machine's own installation (/usr/bin/emacs, or --gnu),
with LD_LIBRARY_PATH unset, as a user runs it, and its version, commit, package
and native compilation are recorded with each run, since an upgrade moves it.
The parity harnesses keep the pinned oracle; only this suite compares against
the installed engine.  The Emacs tree stays pinned: it supplies the same inputs
to every engine (fixture files, ERT test files), not the GNU engine's own Lisp.

Every workload runs in both engines on the same pinned inputs, each engine
in private HOME/XDG/TMPDIR trees, and reports per engine user-space
instructions (`perf stat -e instructions:u', the reliable metric) and wall
time (the median of --runs runs), the Romacs/GNU ratio of each, and the
change of Romacs's figures since the previous saved run.  Results go to
OUT/<commit>/<UTC time>/ as results.json and table.md; the table is printed.

Romacs runs as an installation runs it.  Before anything is measured its
binary is staged beside its Lisp trees and those are compiled ahead of use
(`--compile-lisp-tree', as an installer does), cached per binary and runtime.
Batch rows set ROMACS_COMPILED_LISP=on, which GUI sessions have by default,
and boot from a batch world prepared first, which a user's first batch run
builds once their cache holds a world store.  A workload's own file (micro,
elb) loads by name with an `.elc' beside it, as a package installs, so it runs
from the store too.  A miss reads source without saying so, so each row's note
counts the loads the store's use log confirms, names any read as source, and
says if anything compiled during the row.  The harness's own Lisp, its init and
ERT's test files read as source in both engines; the private package copies
the init and apps rows load read as source too, and those rows say so.
--romacs-source measures the old way: no store, no world, from source.

--neomacs BIN adds a third engine, its own column: the same workloads as it
installs, its dump and its `.elc' files, a workload's own file byte-compiled by
its own compiler.  A row it cannot run says why.  The ERT, init and apps rows
are GNU's and Romacs's alone.

Workloads (--quick runs those marked q):
  batch      GNU's and Romacs's `-Q --batch' on perfbench.el, one process per
             run: instructions are a run's less a run doing only the setup
             (perf/font-lock's method); wall is the body's own float-time.
             startup (q, to the end of command-line), font-lock-ensure on
             fileio.c, subr.el (q) and ORG-NEWS, dired of 5,000 files,
             project-find-regexp over the Emacs repository, vc-log of its
             2,000 commits from the pinned one, occur (q) and how-many (q)
             over xdisp.c, find-file of xdisp.c, describe-function of
             find-file, 20 shell-command-to-string, 20 eshell commands.
  micro      call-path rows, instructions and ns per iteration beyond an
             empty loop: named call (q), closure call (q), mapcar of 1 and
             of 10, dolist + funcall of 10.  GNU runs them byte-compiled,
             Romacs from its compiled store, as each loads compiled Lisp.
  ert        25 GNU ERT files, `-Q --batch -l ert -l FILE -f
             ert-run-tests-batch-and-exit' in both: totals of instructions
             and of per-file wall time, and each engine's expected count.
  elb        GNU ELPA's elisp-benchmarks (pinned tarball), the suite GNU
             evaluates native compilation with: every entry point the suite
             defines in either engine, one process per run, after its own
             garbage-collect; instructions less a setup-only run, wall the
             entry's own.  GNU loads the member files byte-compiled (JIT
             native compilation off) and, when it has native compilation,
             native-compiled at the suite's elb-speed and elb-safety; Romacs
             loads them from its compiled store.  A member missing in an
             engine, or whose run reported an error (the byte compiler's), has no figures there and
             says why; the geometric mean names the members it leaves out.
             Pure Lisp compute, not an editor workload: its own table, never
             in --quick.
  session    what a person feels, in a GUI session on headless labwc: a command
             on a key (dired, occur, project-find-regexp, project-find-file,
             vc-log, vc-diff, vc-dir, M-x shell, org-agenda) and per key editing
             (kill, yank, undo, isearch, Org); then, with Romacs's shipped
             defaults on, what README's lead quotes: find-file of xdisp.c and of
             a generated 15 MB header to the window's final faces,
             vc-print-root-log over upstream master's whole history (limit
             lifted in both; commits, complete, first screen, median key while
             it streams),
             and a theme switch among each engine's own shipped themes, cycled
             and on first visit, none being shipped by both.  Wall runs
             from the key to the result; block is the longest gap between ticks
             of a 5 ms timer, which cannot run while the main VM is busy
             (perfbench-session.el).  A path on a worker VM raises instructions
             and lowers block, so these rows are read by wall and block.
  gui        headless labwc, as the parity harnesses run both engines:
             first-start world build (Romacs only; GNU's dump is built with
             it), warm launch (q) spawn to Romacs's first paint and to the
             first idle moment after emacs-startup-hook (instructions to
             there); per key (q) self-insert, C-f and C-n in a 200-line text
             (whole-process instructions over the keys less the idle rate;
             wall from pre-command-hook to the next idle moment, after
             redisplay); C-x C-f over 5,000 files, fido-vertical in both
             (icomplete-compute-delay 0, so each key's completion is
             synchronous), the keys of `file-1', then the same keys in
             Romacs's default completion (romacs-complete-mode's layer and
             the fzf style; Romacs only).  Romacs's GUI sessions take GNU's
             completion for the comparison rows: fido-vertical, not the
             layer, which takes the root window's last line, and the styles
             without fzf (real_init.GNU_COMPLETION_UI).
  init       by name only (--only init): the real-init profile (real_init.py, from --profile-config or
             $ROMACS_BENCH_PROFILE): init body, from the init's
             first form to its last, GNU against Romacs from source (its
             first launch, which builds the init layer) and from the layer;
             wall only.
  apps       by name only (--only apps): gnosis dashboard open and back to it (gnosis_parity_live.py on
             a copy of the database, --gnosis-db or the profile's), hermes
             dashboard open (hermes_parity_live.py, read-only against the
             running dashboard, the profile's hermes_url): one run each, both engines, with --perf.

Wall times are refused (instructions still reported) for a workload during
which the 1-minute load average exceeded --max-load.  Inputs are pinned:
the Emacs tree is a --shared clone of $ROMACS_GNU_SOURCE checked out at
PIN, its files checked against SHA-256s; the 5,000-file directory is
generated with fixed contents and mtimes and checked by its manifest.

usage: perfbench.py --romacs BIN [--runtime DIR] [--commit SHA] [--quick] [--romacs-source]
                    [--neomacs BIN [--neomacs-commit SHA]]
                    [--runs 3] [--only GROUP|WORKLOAD ...] [--max-load 8]
                    [--out ~/.cache/romacs/perf-bench (ROMACS_PERFBENCH_OUT)]
                    [--previous RESULTS.json] [--gnosis-db COPY --gnosis-db-repo REPO]
                    [--profile-config FILE (ROMACS_BENCH_PROFILE)]
The GNU engine is /usr/bin/emacs (or --gnu).  A binary installed
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
import readme  # noqa: E402
import real_init  # noqa: E402

PIN = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
# Upstream master: the whole-history row counts GNU's commits and no others,
# and PIN carries a branch's on top of them.
UPSTREAM = 'b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b'
TREE = f'emacs-{PIN[:8]}'
PINNED = {'src/fileio.c': '0705ed5b4f93c71a45937d7de737bb54d4ae3c5cef1e565da779f4875f560c92',
          'lisp/subr.el': '55f8319dc1689044b128768c3372db6664397d61afda284bac941c687130bf32',
          'etc/ORG-NEWS': 'd12389d6044a8170912d77536363779b03509075201b0bb814d7e7874ab296b3',
          'src/xdisp.c': '1f2a3df29de9047bfa4b507528c98868cda4d964cbd13711c60ae47d4cbeb346'}
DIR_FILES, DIR_MTIME = 5000, 1767225600  # 2026-01-01T00:00:00Z
DRIVER = HERE / 'perfbench.el'
LOADED_LISP = HERE / 'perfbench-loaded.el'
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
         ('how-many xdisp.c', 'batch', 'how-many-xdisp', True),
         ('find-file xdisp.c', 'batch', 'find-file-xdisp', False),
         ('describe-function find-file', 'batch', 'describe-function', False),
         ('shell-command 20 runs', 'batch', 'shell-command', False),
         ('eshell 20 commands', 'batch', 'eshell-commands', False)]
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
# GNU ELPA's elisp-benchmarks, the suite GNU evaluates native compilation
# with: (version, tarball URL, its SHA-256, member files).
ELB = ('1.16', 'https://elpa.gnu.org/packages/elisp-benchmarks-1.16.tar',
       'e222500d387152bdf3d5a178681071ddf4d9921adf73f97adc3942f3135aad6c', 17)
# (column, engine, how it loads the member files); gnu_native only when
# the GNU engine has native compilation, neomacs only with --neomacs.
ELB_COLUMNS = (('gnu', 'gnu', 'elc'), ('gnu_native', 'gnu', 'eln'), ('romacs', 'romacs', 'store'),
               ('neomacs', 'neomacs', 'elc'))
# An error a run reported instead of signalling: the byte compiler's, or
# the suite's demoted load and run errors.
ELB_ERROR = re.compile(r'(?:^|: )(Error(?: loading| running)?: .*)', re.M)
GROUPS = ('batch', 'micro', 'ert', 'elb', 'session', 'gui', 'init', 'apps')
SESSION_LISP = HERE / 'perfbench-session.el'
SESSION_METRICS = ('wall', 'block', 'instructions', 'first', 'key', 'keys')
# (row, command in perfbench-session.el, whether it ends in a prompt to leave)
SESSION_COMMANDS = [('session dired 5,000 files', 'dired', False),
                    ('session occur xdisp.c', 'occur', False),
                    ('session project-find-regexp', 'project-find-regexp', False),
                    ('session project-find-file', 'project-find-file', True),
                    ('session vc-log 2,000 commits', 'vc-log', False),
                    ('session vc-diff xdisp.c', 'vc-diff', False),
                    ('session vc-dir', 'vc-dir', False),
                    ('session M-x shell, one command', 'shell', False),
                    ('session org-agenda, a month', 'org-agenda', False)]
# What README's lead quotes, in a session as each engine ships: (row, command,
# rounds per --runs, seconds a round may take).  First visits come before the
# cycled rows load their themes.
SESSION_SHIPPED = [('session find-file xdisp.c', 'open-xdisp', 1, 120),
                   ('session find-file generated header', 'open-header', 1, 300),
                   ('session vc-print-root-log whole history', 'vc-history', 1, 300),
                   ('session theme switch, first visit', 'theme-first', 1, 120),
                   ('session theme switch, cycled', 'theme-cycle', 3, 120)]
# Each engine's own shipped themes: (to cycle through, to visit once each).
# Romacs ships none of GNU's, whose modus and tango it replaced with its ports.
GNU_THEMES = (['modus-operandi', 'modus-vivendi', 'tango-dark'], ['wombat', 'leuven', 'adwaita', 'deeper-blue', 'misterioso'])
SESSION_THEMES = {'gnu': GNU_THEMES, 'neomacs': GNU_THEMES,
                  'romacs': (['dracula', 'nord', 'gruvbox-dark'], ['zenburn', 'solarized-dark', 'tokyonight-night',
                                                                   'monokai-classic', 'catppuccin-mocha'])}
HEADER_LINES = 146_000  # about 15 MB, the shape of a generated register header
# (row, Lisp that makes its buffer, Lisp for its check, keys before the row, the row's keys)
SESSION_KEYS = [('session kill-line, per key', '(perfbench-session-text)', '(buffer-size)', [], [parity_live.C('k')] * 40),
                ('session yank, per key', '(perfbench-session-kills)', '(buffer-size)', [], [parity_live.C('y')] * 40),
                ('session undo, per key', '(perfbench-session-undos)', '(buffer-size)', [], [parity_live.C('/')] * 40),
                ('session isearch xdisp.c, per key', '(perfbench-session--xdisp)', 'isearch-string', [parity_live.C('s')],
                 [['-k', 'underscore'] if c == '_' else [c] for c in 'glyph_row']),
                ('session Org self-insert, per key', '(perfbench-session-org)', '(buffer-size)', [], [['a']] * 40),
                ('session Org TAB, per key', '(perfbench-session-org)', '(perfbench-session-visible)', [], [['-k', 'Tab']] * 40)]
# One user's configuration and single packages: the suite judges the corpus
# and general use, so these run only when --only names them.
BY_NAME_ONLY = ('init', 'apps')
# Address space for a neomacs batch run, some fifty times what one needs: on
# one ERT file it grew past 80 GB of memory and swap until the kernel stepped
# in.  Romacs reserves a large arena and is not limited this way.
NEOMACS_MEMORY = 8 << 30
INSTALL_TREES = ('gnu-lisp', 'rlisp', 'etc')  # what an installation ships beside its binary
LISP_JOBS = 8  # compile processes at a time, staging an installation


class Refused(Exception):
    """A workload that cannot run here, with the reason; not a number."""


SESSION_FAILURES = (Refused, TimeoutError, parity_live.LispError, RuntimeError, OSError, ValueError,
                    subprocess.SubprocessError)


# --- pure helpers -------------------------------------------------------------

def median(values):
    values = [v for v in values if v is not None]
    return statistics.median(values) if values else None


def ratio(romacs, gnu):
    """ROMACS over GNU; none when either is missing or not above zero, a count
    its setup or idle floor swallowed, whose ratio would mean nothing."""
    return romacs / gnu if romacs is not None and gnu is not None and romacs > 0 and gnu > 0 else None


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


def loaded_line(text):
    """The files a run printed on its PERFBENCH-LOADED line, or None."""
    for line in text.splitlines():
        if line.startswith('PERFBENCH-LOADED '):
            return parity_live.read_sexp(line[len('PERFBENCH-LOADED '):]) or []
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


def gnu_build(gnu):
    """What GNU, a binary this run compares with, is: an upgrade moves it, so
    each run records its version, commit, package and native compilation."""
    form = ('(princ (format "%S" (list emacs-version emacs-repository-version'
            ' (and (fboundp (quote native-comp-available-p)) (native-comp-available-p) t))))')
    env = {k: v for k, v in os.environ.items() if k != 'LD_LIBRARY_PATH'}
    asked = subprocess.run([gnu, '-Q', '--batch', '--eval', form], env=env, capture_output=True, text=True, timeout=60,
                           stdin=subprocess.DEVNULL)
    version, commit, native = parity_live.read_sexp(asked.stdout.strip().splitlines()[-1])
    return {'path': gnu, 'version': version, 'commit': commit, 'native': bool(native), 'package': package_of(gnu),
            'sha256': hashlib.sha256(Path(gnu).resolve().read_bytes()).hexdigest()}


def package_of(path):
    """The pacman package that owns PATH, as NAME VERSION; None without one."""
    try:
        asked = subprocess.run(['pacman', '-Qo', path], capture_output=True, text=True, timeout=30)
    except OSError:
        return None
    found = re.search(r'is owned by (.+)$', asked.stdout.strip())
    return found[1] if found else None


def geomean(values):
    """The geometric mean of positive VALUES, or None if one is missing."""
    values = list(values)
    if not values or any(v is None or v <= 0 for v in values):
        return None
    return statistics.geometric_mean(values)


ENGINE_NAMES = {'gnu': 'GNU', 'gnu_native': 'GNU native', 'romacs': 'Romacs', 'neomacs': 'neomacs'}
ELB_SUMMARIES = ('elb geometric mean', 'elb median')
RATIOS = ('wall_ratio', 'instructions_ratio', 'native_wall_ratio', 'native_instructions_ratio',
          'neomacs_wall_ratio', 'neomacs_instructions_ratio')


def compiled_loads(samples):
    """(the files SAMPLES loaded, those no artifact ran): each sample's
    :loaded against the sources of the artifacts its run used.  The
    harness's own Lisp and the init it writes read as source in both
    engines and do not count."""
    def real(names):
        return {os.path.realpath(name) for name in names or ()}
    harness = os.path.realpath(HARNESS) + os.sep
    loaded = {name for s in samples for name in real(s['loaded'])
              if not name.startswith(harness) and not name.endswith('/.config/romacs/init.el')}
    missed = {name for s in samples for name in real(s['loaded']) - real(s['compiled']) if name in loaded}
    return loaded, missed


def store_note(samples):
    """What SAMPLES' runs show of the compiled store, for a row's note."""
    loaded, missed = compiled_loads(samples)
    note = f'Romacs: compiled store, {len(loaded) - len(missed)} of {len(loaded)} loads compiled'
    if missed:
        names = sorted(Path(name).name for name in missed)
        more = f' and {len(names) - 6} more' if len(names) > 6 else ''
        note += f', read as source: {", ".join(names[:6])}{more}'
    published = sum(s['published'] for s in samples)
    if published:
        note += f', {published} artifacts compiled during this row'
    unreported = sum(1 for s in samples if s.get('unreported'))
    if unreported:
        note += f', loads not checked in {unreported} of {len(samples)} runs'
    return note


def store_marks(store):
    """Where STORE stands now: how far its use logs run, and the time, set
    back past the file system's coarse clock, whose mtimes lag this one's."""
    return {'logs': {log: log.stat().st_size for log in store.glob('*/use.log')}, 'at': time.time_ns() - 20_000_000}


def store_since(store, marks):
    """What STORE gained since MARKS: `compiled', the sources of the artifacts
    its use logs add, and `published', the artifacts written since.  Only
    a compile writes a store's log besides the process measured, and a
    compile publishes, so published says when `compiled' may not be that
    process's alone.  A log shorter than its mark proves nothing: only the
    maintenance after a compile compacts it, and that counts as a publication
    whatever the maintenance removed."""
    used, compacted = set(), 0
    for log in store.glob('*/use.log'):
        text, mark = log.read_bytes(), marks['logs'].get(log, 0)
        compacted += mark > len(text)
        lines = text[mark:].decode(errors='replace').splitlines() if mark <= len(text) else []
        used |= {line.split('\t', 1)[1] for line in lines if '\t' in line}
    written = sum(1 for path in store.glob('*/*/*.rlc') if path.stat().st_mtime_ns >= marks['at'])
    return {'compiled': used, 'published': written + compacted}


def elb_member(samples, setup):
    """One column's figures from SAMPLES (a member's full runs), less SETUP
    (the setup-only runs' instructions); a missing or failed member has no
    figures and says why.  A count at or below the setup's, a member whose
    work the compiler removed, has no instructions figure and says so."""
    if isinstance(samples, str):
        return {'instructions': None, 'wall': None, 'failed': samples}
    errors = [e for s in samples for e in s['errors']]
    if errors:
        return {'instructions': None, 'wall': None, 'failed': f'reported "{errors[0]}"'}
    instructions = median(s['instructions'] for s in samples) - setup
    made = {'instructions': instructions if instructions > 0 else None,
            'wall': median(s['seconds'] for s in samples), 'setup_instructions': setup}
    if instructions <= 0:
        made['note'] = f'instructions not above the setup run\'s ({instructions:+.0f})'
    return made


def elb_note(engines):
    return '; '.join(f'{ENGINE_NAMES[c]}: {v.get("failed") or v["note"]}'
                     for c, v in engines.items() if v.get('failed') or v.get('note'))


def elb_geomean_row(rows, columns, runs):
    """The geometric mean of GNU's .elc column and Romacs's over the members
    both ran; the members left out are named.  No other column has one:
    native compilation reduces some members to almost nothing, and one such
    ratio would decide a mean, so those columns have `elb_median_row'."""
    columns = [c for c in columns if c in ('gnu', 'romacs')]
    complete = [r for r in rows if not any(r[c].get('failed') for c in columns)]
    left_out = [r['row'].removeprefix('elb ') for r in rows if r not in complete]
    engines = {c: {m: geomean(r[c][m] for r in complete) for m in ('instructions', 'wall')} for c in columns}
    note = f'against GNU .elc, over the {len(complete)} of {len(rows)} members both ran'
    if left_out:
        note += f'; left out: {", ".join(left_out)}'
    without = [f'{ENGINE_NAMES[c]} {m}' for c in columns for m in ('instructions', 'wall') if engines[c][m] is None]
    if complete and without:
        note += f'; no mean of {", ".join(without)}: a member has no figure there'
    made = row('elb geometric mean', 'elb', 'run', engines['gnu'], engines['romacs'], runs, note)
    made['load'] = max((r.get('load', 0) for r in rows), default=0)
    return made


def elb_median_row(rows, runs):
    """The row `with_medians' fills in, once the members have their ratios."""
    made = row('elb median', 'elb', 'run', None, None, runs, '')
    made['load'] = max((r.get('load', 0) for r in rows), default=0)
    return made


def with_medians(rows):
    """ROWS with `elb median' given each ratio's median over the members that
    have it, the summary no single member decides."""
    summary = next((r for r in rows if r['row'] == 'elb median'), None)
    if summary is None:
        return rows
    members = [r for r in rows if r['group'] == 'elb' and r['row'] not in ELB_SUMMARIES]
    values = {key: [r[key] for r in members if r.get(key) is not None] for key in RATIOS}
    summary.update({key: median(found) for key, found in values.items()})
    counted = [f'{name} {len(values[key])}' for name, key in (('GNU .elc', 'instructions_ratio'),
                                                                ('GNU native', 'native_instructions_ratio'),
                                                                ('neomacs', 'neomacs_instructions_ratio')) if values[key]]
    summary['note'] = f'median of the per-member ratios, members with instructions figures: {", ".join(counted)}'
    return rows


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


def run(argv, env, cwd, timeout, perf_out=None, memory=None):
    """ARGV in its own process group (killed whole on timeout), under perf
    stat when PERF_OUT is a path, with at most MEMORY bytes of address space
    when given: (stdout+stderr text, wall, instructions)."""
    if perf_out is not None:
        argv = ['perf', 'stat', '-x,', '-e', 'instructions:u', '-o', str(perf_out), '--', *argv]
    if memory:  # outermost, so perf does not count it; no Python runs in the child, as this has threads
        argv = ['prlimit', f'--as={memory}', '--', *argv]
    started = time.time()
    process = subprocess.Popen(argv, env=env, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True, errors='replace', start_new_session=True)
    try:
        output, _ = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate()
        raise Refused(f'timed out after {timeout}s: {" ".join(Path(str(a)).name for a in argv[-4:])}')
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


# --- the installation ---------------------------------------------------------

def stage_install(cache, binary, runtime):
    """BINARY as an installer leaves it, RUNTIME's trees beside it and its
    Lisp compiled ahead of use: (the staged binary, the compile's summary).
    Cached per binary and runtime; a staged installation of any other goes,
    each being a few hundred MB.  A live installation's store is never
    written, nor is this done while anything is measured."""
    digest = hashlib.sha256(binary.read_bytes())
    for tree in INSTALL_TREES:
        for path in sorted(p for p in (runtime / tree).rglob('*') if p.is_file()):
            digest.update(str(path.relative_to(runtime)).encode() + path.read_bytes())
    parent = cache / 'install'
    root = parent / digest.hexdigest()[:16]
    staged, summary = root / 'romacs-vm-gui', root / 'compile-lisp.json'
    if summary.exists():
        return staged, json.loads(summary.read_text())
    parent.mkdir(exist_ok=True)
    for other in parent.iterdir():
        shutil.rmtree(other)
    root.mkdir()
    for tree in INSTALL_TREES:
        subprocess.run(['cp', '-a', '--reflink=auto', str(runtime / tree), str(root)], check=True, timeout=600)
    shutil.copy2(binary, staged)
    home = root / 'home'
    home.mkdir()
    started = time.time()
    print(f'staging an installation in {root}, compiling its Lisp', flush=True)
    done = subprocess.run([str(staged), '--compile-lisp-tree', '--jobs', str(LISP_JOBS)], cwd=root, timeout=3600,
                          env={'PATH': os.environ['PATH'], 'HOME': str(home), 'LANG': 'C.UTF-8',
                               'ROMACS_RUNTIME_ROOT': str(root), 'ROMACS_LISP_DIR': str(root / 'gnu-lisp')},
                          stdin=subprocess.DEVNULL, capture_output=True, text=True)
    # 2: complete, with files that failed to compile and load as source.
    if done.returncode not in (0, 2):
        raise SystemExit(f'--compile-lisp-tree failed ({done.returncode}):\n{done.stderr[-4000:]}')
    report = json.loads(done.stdout.strip().splitlines()[-1])
    made = {'files': report['files'], 'not_compiled': len(report['source_only']) + len(report['errors']),
            'seconds': round(time.time() - started, 1), 'store_bytes': report['store_bytes']}
    summary.write_text(json.dumps(made))
    return staged, made


def prepare_batch_world(ctx, env):
    """The batch world ENV's HOME boots from.  A user's is built by their
    first batch run once their cache holds a world store, which the GUI's
    first launch makes; a fresh HOME makes none, so perfbench makes the store
    and prepares it here.  A detached builder makes it, and a batch session
    does not say what it booted from, so it is ready when a boot's
    instructions fall to a fraction of the first's, a boot from source: that
    fraction, which this returns, tells a world from a source boot later."""
    store = Path(env['XDG_CACHE_HOME']) / 'romacs/worlds'
    store.mkdir(mode=0o700, parents=True)
    store.parent.chmod(0o700)
    ceiling = boot_instructions(ctx, env) / 4
    wait_for_world(ctx, env, ceiling)
    return ceiling


def boot_instructions(ctx, env):
    """The instructions of one batch boot in ENV."""
    stat = ctx.scratch('.stat')
    _, _, instructions = run([ctx.engines['romacs'], '-Q', '--batch', '--eval', 'nil'], env, ctx.fixtures, 300, stat)
    stat.unlink()
    return instructions


def wait_for_world(ctx, env, ceiling):
    """Return once a boot in ENV is a boot from a world, under CEILING."""
    deadline = time.time() + 600
    while time.time() < deadline:
        instructions = boot_instructions(ctx, env)
        if instructions and instructions < ceiling:
            return
        time.sleep(5)
    raise SystemExit('no batch world after 10 minutes; a run would boot from source and not say so')


# --- batch workloads ----------------------------------------------------------

class Batch:
    """Both engines' `-Q --batch' on the driver, in one private tree each."""

    def __init__(self, ctx):
        self.ctx = ctx
        self.env = {name: private_env(ctx.work, f'batch-{name}', extra) for name, extra in ctx.engine_env.items()}
        self.world_ceiling = prepare_batch_world(ctx, self.env['romacs']) if ctx.store else None

    def check_world(self, engine):
        """Refuse the run just made if it left Romacs's batch world stale, as a
        new ~/.emacs.d does: the next boot is from source, with a builder
        counted in it, so nothing after it is a warm run until the world is
        back, which this waits for."""
        if engine != 'romacs' or not self.world_ceiling:
            return
        if boot_instructions(self.ctx, self.env['romacs']) >= self.world_ceiling:
            wait_for_world(self.ctx, self.env['romacs'], self.world_ceiling)
            raise Refused('romacs: the run left its batch world stale')

    def once(self, engine, workload, skip=False, extra=None):
        env = {**self.env[engine], 'PERFBENCH_WORKLOAD': workload, 'PERFBENCH_FIXTURES': str(self.ctx.fixtures),
               **(extra or {})}
        if skip:
            env['PERFBENCH_SKIP'] = '1'
        stat = self.ctx.scratch('.stat')
        argv = [self.ctx.engines[engine], '-Q', '--batch', '-l', str(DRIVER)]
        marks = self.ctx.store_marks()
        output, wall, instructions = run(argv, env, self.ctx.fixtures, 900, stat, self.ctx.memory.get(engine))
        stat.unlink()
        self.check_world(engine)
        result = perfbench_line(output)
        if result is None or instructions is None:
            self.ctx.log(f'{engine} {workload}', output)
            last = next((line for line in reversed(output.splitlines()) if line.strip()), '')
            raise Refused(f'{engine}: no result from {workload}: {last[:160]!r} (see logs)')
        return dict(result, instructions=instructions, process_wall=wall, output=output,
                    **self.ctx.store_since(marks))

    def figures(self, engine, workload):
        runs = self.ctx.runs
        # What a first use compiles is no part of a warm row; its note says so.
        first = self.once(engine, workload) if engine == 'romacs' and self.ctx.store else {}
        full = [self.once(engine, workload) for _ in range(runs)]
        if workload == 'startup':
            return {'instructions': median(r['instructions'] for r in full),
                    'wall': median(r['process_wall'] for r in full), 'check': None, 'samples': full,
                    'first_use': first.get('published', 0)}
        skipped = [self.once(engine, workload, skip=True) for _ in range(runs)]
        return {'instructions': median(r['instructions'] for r in full) - median(r['instructions'] for r in skipped),
                'wall': median(r['seconds'] for r in full), 'check': full[0]['check'],
                'setup_instructions': median(r['instructions'] for r in skipped), 'samples': full,
                'first_use': first.get('published', 0)}

    def workload(self, name, workload):
        engines = {e: self.ctx.attempt(e, lambda e=e: self.figures(e, workload)) for e in self.ctx.compared}
        samples = {e: figures.pop('samples', None) for e, figures in engines.items()}
        note = 'process to exit' if workload == 'startup' else 'less its setup'
        for engine, figures in engines.items():
            if figures.get('instructions') is not None and figures['instructions'] <= 0:
                note += f'; {ENGINE_NAMES[engine]}: instructions not above the setup run\'s ({figures["instructions"]:+.0f})'
                figures['instructions'] = None
        if engines['romacs'].get('first_use'):
            note += f'; Romacs: {engines["romacs"]["first_use"]} artifacts compiled on first use, before these runs'
        made = row(name, 'batch', 'run', engines['gnu'], engines['romacs'], self.ctx.runs,
                   f'{check_note(engines, note)}; {self.ctx.romacs_note(samples["romacs"])}')
        return with_neomacs(made, engines)


def with_neomacs(made, engines):
    """MADE with neomacs's figures from ENGINES, when it ran with them, and
    what its note should say of them."""
    figures = engines.get('neomacs')
    if figures is None:
        return made
    made['neomacs'] = figures
    if figures.get('failed'):
        made['note'] += f'; neomacs: {figures["failed"]}'
    elif figures.get('check') != (engines.get('gnu') or {}).get('check'):
        made['note'] += f'; neomacs result differs: {figures.get("check")!r}'
    return made


def check_note(engines, note=''):
    gnu, romacs = engines['gnu'].get('check'), engines['romacs'].get('check')
    if gnu != romacs:
        note = f'{note}; result differs: GNU {gnu!r}, Romacs {romacs!r}'.lstrip('; ')
    return note


def micro_rows(ctx, batch):
    """The call-path rows: per iteration, beyond the empty loop."""
    selected = [m for m in MICRO if m[2] or not ctx.quick]
    directory = ctx.cache / 'micro' / ctx.gnu_build['sha256'][:12]  # bytecode is the engine's own
    directory.mkdir(parents=True, exist_ok=True)
    samples = {}
    for name, body, _ in selected:
        text = MICRO_HEAD.replace('BODY', body)
        source = directory / f'{hashlib.sha256(text.encode()).hexdigest()[:12]}.el'
        source.write_text(text)
        compiled = source.with_suffix('.elc')
        if not compiled.exists():
            run([ctx.engines['gnu'], '-Q', '--batch', '--eval', '(setq byte-compile-warnings nil)',
                 '-f', 'batch-byte-compile', str(source)], batch.env['gnu'], directory, 300)
        # By name, the file beside its .elc, an installed package's load.
        loaded = {'gnu': lambda: compiled, 'romacs': lambda: source.with_suffix('') if ctx.store else source,
                  'neomacs': lambda: neomacs_compiled(ctx, batch, source)}

        def runs(engine):
            extra = {'PERFBENCH_MICRO': str(loaded[engine]()), 'PERFBENCH_N': str(MICRO_N)}
            if engine == 'romacs' and ctx.store:  # its compile is no part of what the rows count
                batch.once('romacs', 'micro', skip=True, extra=extra)
            return [batch.once(engine, 'micro', extra=extra) for _ in range(ctx.runs)]
        samples[name] = {engine: ctx.attempt(engine, lambda e=engine: runs(e)) for engine in ctx.compared}

    def per_iteration(name, engine, key):
        return (median(s[key] for s in samples[name][engine])
                - median(s[key] for s in samples['loop'][engine])) / MICRO_N

    def figures(name, engine):
        failed = next((samples[n][engine] for n in ('loop', name) if isinstance(samples[n][engine], dict)), None)
        return failed or {'instructions': per_iteration(name, engine, 'instructions'),
                          'wall': per_iteration(name, engine, 'seconds'), 'check': None}
    rows = []
    for name, body, _ in selected[1:]:
        engines = {engine: figures(name, engine) for engine in ctx.compared}
        rows.append(with_neomacs(row(f'micro {name}', 'micro', 'iteration', engines['gnu'], engines['romacs'], ctx.runs,
                                     f'{body}; GNU compiled; {ctx.romacs_note(samples[name]["romacs"])}'), engines))
    return rows


def neomacs_compiled(ctx, batch, source):
    """SOURCE byte-compiled by neomacs's own compiler, beside a copy of it."""
    directory = ctx.work / 'micro-neomacs'
    directory.mkdir(exist_ok=True)
    copy = directory / source.name
    shutil.copy(source, copy)
    run([ctx.engines['neomacs'], '-Q', '--batch', '--eval', '(setq byte-compile-warnings nil)',
         '-f', 'batch-byte-compile', str(copy)], batch.env['neomacs'], directory, 300, memory=ctx.memory.get('neomacs'))
    return copy.with_suffix('.elc')


def ert_pass(ctx, batch, engine, hung):
    """One pass of the 25 files in ENGINE: totals, counts and the files without
    a result.  A file refused once joins HUNG and is not run again: it costs
    the engine that file's result, not the row."""
    instructions, wall, ran, expected, failed, loads = 0, 0.0, 0, 0, [], []
    report = '(add-hook (quote kill-emacs-hook) (lambda () (princ (format "PERFBENCH-LOADED %S\\n" (perfbench-loaded)))))'
    tests = os.path.realpath(ctx.fixtures) + os.sep
    for name in ERT_FILES:
        if name in hung:
            failed.append(name)
            continue
        path = ctx.fixtures / TREE / 'test' / name
        stat = ctx.scratch('.stat')
        marks = ctx.store_marks()
        try:
            output, seconds, count = run([ctx.engines[engine], '-Q', '--batch', '-l', str(LOADED_LISP), '--eval', report,
                                          '-l', 'ert', '-l', str(path), '-f', 'ert-run-tests-batch-and-exit'],
                                         batch.env[engine], path.parent, 900, stat, ctx.memory.get(engine))
            batch.check_world(engine)
        except Refused as error:
            hung.add(name)
            failed.append(name)
            ctx.log(f'ert {engine} {name}', str(error))
            continue
        stat.unlink()
        reported = loaded_line(output)
        loads.append({'loaded': [f for f in reported or () if not os.path.realpath(f).startswith(tests)],
                      'unreported': reported is None, **ctx.store_since(marks)})
        counts = ert_counts(output)
        if count is None or counts is None:
            failed.append(name)
            ctx.log(f'ert {engine} {name}', output)
            continue
        instructions, wall = instructions + count, wall + seconds
        ran, expected = ran + counts[0], expected + counts[1]
    return {'instructions': instructions, 'wall': wall, 'check': f'{expected}/{ran} expected', 'failed': failed,
            'loads': loads}


def ert_row(ctx, batch):
    """25 GNU ERT files, the same command in both engines: medians of
    --runs passes, withheld when any pass lacks a file's result."""
    engines, passes_of = {}, {}
    for engine in ('gnu', 'romacs'):
        hung = set()
        passes = passes_of[engine] = [ert_pass(ctx, batch, engine, hung) for _ in range(ctx.runs)]
        failed = sorted({name for p in passes for name in p['failed']})
        totals = {k: median(p[k] for p in passes) for k in ('instructions', 'wall')}
        engines[engine] = {**({k: None for k in totals} if failed else totals),
                           'partial': totals if failed else None,
                           'check': passes[0]['check'], 'files_without_result': failed}
    note = f'GNU {engines["gnu"]["check"]}, Romacs {engines["romacs"]["check"]}; wall summed per file; ' \
           + ctx.romacs_note([s for p in passes_of['romacs'] for s in p['loads']]) + '; test files read as source in both'
    if engines['gnu']['files_without_result'] or engines['romacs']['files_without_result']:
        note += f'; totals withheld, files without a result: GNU {engines["gnu"]["files_without_result"]},' \
                f' Romacs {engines["romacs"]["files_without_result"]}'
    return row('ERT 25 files', 'ert', 'total', engines['gnu'], engines['romacs'], ctx.runs, note)


def elb_tarball(cache):
    """The pinned elisp-benchmarks tarball, fetched once, checked every run."""
    version, url, digest, _ = ELB
    path = cache / 'elb' / f'elisp-benchmarks-{version}.tar'
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        staging = path.with_suffix('.part')
        with urllib.request.urlopen(url, timeout=60) as response:
            staging.write_bytes(response.read())
        staging.rename(path)
    if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        raise SystemExit(f'{path} does not match its pinned SHA-256')
    return path


def gnu_native_comp(ctx, batch):
    output, _, _ = run([ctx.engines['gnu'], '-Q', '--batch', '--eval',
                        '(princ (if (and (fboundp (quote native-comp-available-p)) (native-comp-available-p))'
                        ' "native-comp yes" "native-comp no"))'], batch.env['gnu'], ctx.work, 60)
    return 'native-comp yes' in output


def elb_rows(ctx, batch):
    """GNU ELPA's elisp-benchmarks, every entry point the suite defines in
    either engine: GNU byte-compiled (and native-compiled when it can),
    Romacs from source.  Instructions are a run's less a setup-only run."""
    version, _, _, count = ELB
    tarball = elb_tarball(ctx.cache)
    ctx.elb_native = gnu_native_comp(ctx, batch)
    columns = [c for c in ELB_COLUMNS if c[1] in ctx.engines and (c[0] != 'gnu_native' or ctx.elb_native)]
    package_of = lambda column: ctx.work / f'elb-{column}' / f'elisp-benchmarks-{version}'  # noqa: E731
    prepared = {}
    for column, engine, mode in columns:
        root = ctx.work / f'elb-{column}'
        root.mkdir()
        subprocess.run(['tar', '-xf', str(tarball), '-C', str(root)], check=True, timeout=60)
        package = package_of(column)
        found = len(list((package / 'benchmarks').glob('*.el')))
        if found != count:
            raise Refused(f'elisp-benchmarks {version} has {found} member files, not {count}')
        if mode == 'store' and not ctx.store:
            mode = 'source'
        if mode == 'store':  # the .elc an installed package has beside each file
            for compiled in package_of('gnu').rglob('*.elc'):
                shutil.copy(compiled, package / compiled.relative_to(package_of('gnu')))
        env = {'PERFBENCH_ELB_DIR': str(package), 'PERFBENCH_ELB_MODE': mode, 'PERFBENCH_ELN': str(root / 'eln')}
        prepared[column] = elb_column(batch, engine, env, ctx.runs)
    members = sorted({e for p in prepared.values() for e in p['entries']})
    if not members:
        raise Refused('; '.join(f'{ENGINE_NAMES[c]}: {p["failed"]}' for c, p in prepared.items()))
    rows = []
    for member in members:
        watch = LoadWatch()
        samples = watch.during(lambda: {c: elb_samples(ctx, batch, prepared[c], member) for c in prepared})
        engines = {c: elb_member(samples[c], prepared[c].get('setup')) for c in prepared}
        ran = samples['romacs'] if isinstance(samples['romacs'], list) else None
        made = row(f'elb {member}', 'elb', 'run', engines['gnu'], engines['romacs'], ctx.runs,
                   '; '.join(filter(None, [elb_note(engines), ran and ctx.romacs_note(ran)])))
        made.update({c: engines[c] for c in ('gnu_native', 'neomacs') if c in engines})
        made['load'] = watch.peak
        rows.append(made)
    return rows + [elb_geomean_row(rows, list(prepared), ctx.runs), elb_median_row(rows, ctx.runs)]


def elb_column(batch, engine, env, runs):
    """ENGINE's suite loaded as ENV says: its entry names and its setup-only
    runs' instructions; or, when it refuses to compile or load the suite,
    why, so the other columns still have their figures."""
    try:
        entries = batch.once(engine, 'elb-prepare', extra=env)['check']
        setup = median(batch.once(engine, 'elb', skip=True, extra=env)['instructions'] for _ in range(runs))
    except Refused as error:
        return {'engine': engine, 'env': env, 'entries': [], 'failed': f'refused: {error}'}
    return {'engine': engine, 'env': env, 'entries': entries, 'setup': setup}


def elb_samples(ctx, batch, column, member):
    """--runs runs of MEMBER in COLUMN, each with the errors it reported;
    or why it has none."""
    if column.get('failed'):
        return column['failed']
    if member not in column['entries']:
        return f'no elb-{member}-entry after loading the suite'
    extra = {**column['env'], 'PERFBENCH_ELB_ENTRY': member}
    samples = []
    for _ in range(ctx.runs):
        try:
            sample = batch.once(column['engine'], 'elb', extra=extra)
        except Refused as error:
            return f'refused: {error}'
        errors = ELB_ERROR.findall(sample['output'])
        if errors:
            batch.ctx.log(f'elb {column["engine"]} {column["env"]["PERFBENCH_ELB_MODE"]} {member}', sample['output'])
        samples.append({**sample, 'errors': errors})
    return samples


# --- GUI workloads ------------------------------------------------------------

def gui_init(engine, idle_file=None, shipped=False):
    """The GUI init: GNU's --loadup settings (Romacs's own defaults off in
    Romacs, unless SHIPPED: it then starts as a user's does), the probe and
    perfbench-gui.el; with IDLE_FILE, write the first idle moment after startup
    there and exit."""
    idle = ('(add-hook (quote emacs-startup-hook) (lambda () (run-with-idle-timer 0 nil'
            f' (lambda () (perfbench-gui-write-loaded {parity_live.lisp_string(str(idle_file) + "-loaded")})'
            f' (perfbench-gui-stamp {parity_live.lisp_string(str(idle_file))}) (kill-emacs 0)))))\n'
            if idle_file else '')
    return (';; -*- lexical-binding: t -*-\n'
            f'(load {parity_live.lisp_string(str(GUI_LISP))} nil t)\n'
            + (('' if shipped else real_init.GNU_COMPLETION_UI + parity_live.ROMACS_DEFAULTS_OFF) if engine == 'romacs'
               else real_init.gnu_settings())
            + '(setq inhibit-startup-screen t)\n' + idle
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
                ctx.runs, 'Romacs only: --loadup --world-build in a fresh store; a world build reads source,'
                ' never the compiled store')
    launches = {engine: ctx.attempt(engine, lambda e=engine: launch_samples(ctx, e)) for engine in ctx.compared}
    figures = {e: launches[e] if isinstance(launches[e], dict) else
               {'instructions': median(s['instructions'] for s in launches[e]),
                'wall': median(s['idle'] for s in launches[e]), 'check': None} for e in ctx.compared}
    idle = with_neomacs(row('warm launch to idle', 'gui', 'run', figures['gnu'], figures['romacs'], ctx.runs,
                            'spawn to the first idle after emacs-startup-hook; instructions to exit there; '
                            + ctx.romacs_note(launches['romacs'])), figures)
    paint = row('warm launch to first paint', 'gui', 'run', None,
                {'instructions': None, 'wall': median(s['paint'] for s in launches['romacs'])}, ctx.runs,
                'Romacs only (its --trace-latency first-paint); GNU has no paint mark; '
                + ctx.romacs_note(launches['romacs']))
    return [world, paint, idle]


def session_repo(ctx):
    """A private clone of the pinned tree with src/xdisp.c changed, for the
    rows that write or need a change, and the Org file the agenda reads."""
    repo = ctx.work / 'session-repo'
    git('clone', '-q', '--shared', str(ctx.fixtures / TREE), str(repo))
    with (repo / 'src/xdisp.c').open('a') as out:
        out.write('/* perfbench */\n')
    agenda = ctx.work / 'agenda.org'
    agenda.write_text(''.join(f'* TODO item {n}\n  SCHEDULED: <2026-01-{n % 28 + 1:02}>\n' for n in range(2000)))
    header = ctx.work / 'generated.h'
    header.write_text(generated_header(HEADER_LINES))
    ctx.header_bytes = header.stat().st_size
    history = ctx.work / 'session-history'
    git('clone', '-q', '--shared', '--no-checkout', str(ctx.fixtures / TREE), str(history))
    git('-c', 'advice.detachedHead=false', 'checkout', '-q', UPSTREAM, cwd=history)
    ctx.history_commits = int(git('rev-list', '--count', 'HEAD', cwd=history))
    return repo, agenda, header, history


def generated_header(lines):
    """A generated C register header of LINES lines, the same on every run."""
    def line(n):
        name = f'#define REG_{n // 12:06}_CNTL__FIELD_{n % 12:02}_MASK'
        return f'//REG_{n // 12:06}_CNTL\n' if n % 12 == 0 else f'{name:<96}0x{(n * 2654435761) % 2 ** 32:08X}L\n'
    return ''.join(line(n) for n in range(lines))


def session_command(session, name, leave, rate, rounds, timeout=120):
    """Wall, block and instructions of command NAME, the median of ROUNDS, each
    allowed TIMEOUT seconds, and what it showed first and how its keys fared."""
    samples, checks, detail = [], [], None
    for n in range(rounds):
        done = session.root / f'work/{name}-{n}'
        session.ask(f'(perfbench-session-arm (quote {name}) {parity_live.lisp_string(str(done))})', 120)
        time.sleep(.5)

        def act():
            session.engine.keys(['-k', 'F5'])
            parity_live.wait(done.exists, timeout, f'{name} result', session.engine.process, .01)
        instructions, elapsed = session.counted(act, timeout)
        seconds, block, first, key, keys = parity_live.read_sexp(done.read_text())
        samples.append({'wall': seconds, 'block': block, 'first': first, 'key': key, 'keys': keys,
                        'instructions': None if instructions is None else instructions - rate * elapsed})
        if leave:
            session.engine.keys(parity_live.C('g'))
        check, detail = session.ask(f'(perfbench-session-finish (quote {name}))', 120)
        checks.append(check)
        time.sleep(.5)
    return session_medians(samples, checks) | {'detail': detail}


def session_keys(session, setup, check_form, before, keys, rate, rounds):
    """Per-key wall, block and instructions, the median of ROUNDS."""
    samples, checks = [], []
    for _ in range(rounds):
        session.ask(f'(progn {setup} t)', 120)
        time.sleep(1)
        for group in before:
            session.engine.keys(group)
        time.sleep(.5)
        session.ask('(perfbench-session-reset)')
        figures = session.phase(keys, .1, rate)
        figures['block'] = session.ask('perfbench-session-worst-gap')
        ran = session.ask('perfbench-gui-commands')
        if ran != len(keys):
            raise Refused(f'{ran} of {len(keys)} keys ran')
        checks.append(str(session.ask(f'(with-current-buffer (window-buffer (selected-window)) {check_form})')))
        session.engine.keys(parity_live.C('g'))
        session.engine.keys(parity_live.C('g'))
        samples.append(figures)
    session.ask('(perfbench-session-abort nil)')
    return session_medians(samples, checks)


def session_medians(samples, checks):
    """SAMPLES' medians, when every round made the same thing, or a refusal:
    a round that did something else is no sample of this row."""
    if len(set(map(str, checks))) > 1:
        raise Refused(f'rounds differ: {checks}')
    return {metric: median(s.get(metric) for s in samples) for metric in SESSION_METRICS} | {'check': checks[0]}


def session_start(session, ctx, engine, inputs, comparing):
    """SESSION with its Lisp loaded and its inputs set; COMPARING, in fido-vertical."""
    repo, agenda, header, history = inputs
    cycle, visits = SESSION_THEMES[engine]
    symbols = lambda names: f"'({' '.join(names)})"  # noqa: E731
    session.ask(f'(progn (load {parity_live.lisp_string(str(SESSION_LISP))} nil t t)'
                f' (setq perfbench-session-repo {parity_live.lisp_string(str(repo))}'
                f' perfbench-session-directory {parity_live.lisp_string(str(ctx.fixtures / "d5000") + "/")}'
                f' perfbench-session-agenda {parity_live.lisp_string(str(agenda))}'
                f' perfbench-session-header {parity_live.lisp_string(str(header))}'
                f' perfbench-session-history {parity_live.lisp_string(str(history))}'
                f' perfbench-session-cycle {symbols(cycle)} perfbench-session-visits {symbols(visits)}'
                ' auto-save-default nil create-lockfiles nil make-backup-files nil)'
                f' {FIDO + " (setq icomplete-compute-delay 0)" if comparing else ""} (blink-cursor-mode -1) t)')
    time.sleep(2)
    return session.idle_rate()


def session_group_figures(ctx, engine, inputs):
    """Every session row of ENGINE, in a session for the comparison rows and one
    as it ships for the lead's: a row that fails says so and costs that row alone."""
    figures = {}
    session = Session(ctx, engine)
    try:
        rate = session_start(session, ctx, engine, inputs, True)
        for name, command, leave in SESSION_COMMANDS:
            figures[name] = session_row(session, name, lambda c=command, l=leave: session_command(session, c, l, rate, ctx.runs),
                                        command)
        for name, setup, check, before, keys in SESSION_KEYS:
            figures[name] = session_row(session, name, lambda s=setup, c=check, b=before, k=keys:
                                        session_keys(session, s, c, b, k, rate, ctx.runs))
        if engine == 'romacs' and not session.stopped:
            figures['romacs_note'] = session.romacs_note()
    finally:
        session.stop()
    session = Session(ctx, engine, shipped=True)
    try:
        rate = session_start(session, ctx, engine, inputs, False)
        for name, command, repeat, timeout in SESSION_SHIPPED:
            # A first visit happens once per theme, however many --runs there are.
            rounds = min(ctx.runs, len(SESSION_THEMES[engine][1])) if command == 'theme-first' else ctx.runs * repeat
            figures[name] = session_row(session, name, lambda c=command, r=rounds, t=timeout:
                                        session_command(session, c, False, rate, r, t), command)
        if engine == 'romacs' and not session.stopped:
            figures['shipped_note'] = session.romacs_note()
    finally:
        session.stop()
    return figures


def session_row(session, name, function, command=None):
    """FUNCTION's figures, or why it has none, the session left ready for the
    next.  A session that stops answering stays stopped: the rows after it say
    so at once, and what it already measured stands."""
    if session.stopped:
        return {'wall': None, 'block': None, 'instructions': None, 'failed': f'not run: {session.stopped}'}
    print(f'    {session.engine.name} {name}', flush=True)
    try:
        return function()
    except SESSION_FAILURES as error:
        try:
            for _ in range(3):
                session.engine.keys(parity_live.C('g'))
            session.ask(f'(perfbench-session-abort (quote {command or "nil"}))', 15)
        except SESSION_FAILURES:
            session.stopped = f'the session stopped answering in {name.removeprefix("session ")}'
        return {'wall': None, 'block': None, 'instructions': None, 'failed': f'not run: {str(error)[:160]}'}


def session_rows(ctx):
    """The session group: one session per engine, the rows from each."""
    inputs = session_repo(ctx)
    results = {engine: ctx.attempt(engine, lambda e=engine: session_group_figures(ctx, e, inputs)) for engine in ctx.compared}
    stopped = 'Romacs: loads not checked, the session stopped answering'
    notes = {'comparison': results['romacs'].pop('romacs_note', stopped), 'shipped': results['romacs'].pop('shipped_note', stopped)}
    shipped = [c[0] for c in SESSION_SHIPPED]
    names = [c[0] for c in SESSION_COMMANDS] + [k[0] for k in SESSION_KEYS] + shipped
    rows = []
    for name in names:
        romacs_note = notes['shipped' if name in shipped else 'comparison']
        if 'theme switch' in name:
            romacs_note += '; each engine on its own shipped themes'
        engines = {e: results[e] if results[e].get('failed') else results[e][name] for e in ctx.compared}
        unit = 'key' if name.endswith('per key') else 'run'
        note = '; '.join(f'{ENGINE_NAMES[e]}: {engines[e]["failed"]}' for e in ('gnu', 'romacs') if engines[e].get('failed'))
        made = row(name, 'session', unit, engines['gnu'], engines['romacs'], ctx.runs,
                   '; '.join(filter(None, [note, check_note(engines), romacs_note])))
        rows.append(with_neomacs(made, engines))
    return rows


def launch_samples(ctx, engine):
    """ENGINE's warm launches, --runs of them after one that warms the page cache."""
    root, home, env = parity_live.private_tree(engine, ctx.work, ctx.tools)
    env.update(ctx.engine_env[engine])
    idle_file = root / 'idle'
    init = gui_init(engine, idle_file)
    if engine == 'romacs':
        (home / '.config/romacs').mkdir()
        (home / '.config/romacs/init.el').write_text(init)
        env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
        parity_live.prepare_world(ctx.engines['romacs'], env, root / 'work')
        argv = [ctx.engines['romacs'], '--loadup', '--trace-latency']
    else:
        (root / 'init.el').write_text(init)
        argv = [ctx.engines[engine], '-Q', '--font', 'Iosevka-14', '-l', str(root / 'init.el')]
    samples = []
    for n in range(ctx.runs + 1):  # the first launch warms the page cache
        fresh_session_dirs(root)
        if idle_file.exists():
            idle_file.unlink()
        stat = root / f'launch-{n}.stat'
        marks = ctx.store_marks()
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
                        'instructions': stat_instructions(stat.read_text()),
                        'loaded': parity_live.read_sexp(Path(f'{idle_file}-loaded').read_text()),
                        **ctx.store_since(marks)})
    return samples[1:]


class Session:
    """One engine's GUI session on the probe, measuring key phases."""

    def __init__(self, ctx, engine, shipped=False):
        root, home, env = parity_live.private_tree(engine, ctx.work, ctx.tools)
        env.update(ctx.engine_env[engine])
        init = gui_init(engine, shipped=shipped)
        start = {'gnu': parity_live.gnu_engine,
                 'romacs': lambda *a: parity_live.romacs_engine(ctx.engines['romacs'], *a),
                 'neomacs': lambda *a: parity_live.neomacs_engine(ctx.engines.get('neomacs'), *a)}[engine]
        self.ctx, self.marks, self.stopped = ctx, ctx.store_marks(), None
        self.engine = start(root, home, env, init, None, 120, ctx.tools)
        self.root = root

    def romacs_note(self):
        """What this session shows of the compiled store."""
        return self.ctx.romacs_note([{'loaded': self.ask('(perfbench-loaded)'), **self.ctx.store_since(self.marks)}])

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
                                   # A thread's name is cut at 15 bytes, perhaps mid-character.
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, errors='replace')
        try:
            time.sleep(.25)
            act()
            time.sleep(.25)
        finally:
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
    results = {engine: ctx.attempt(engine, lambda e=engine: session_figures(ctx, e, text, keys, gap, rounds))
               for engine in ctx.compared}
    romacs_note = results['romacs'].pop('romacs_note')

    def neomacs(name):
        figures = results.get('neomacs')
        return {} if figures is None else {'gnu': results['gnu'][name],
                                           'neomacs': figures if figures.get('failed') else figures[name]}
    rows = [with_neomacs(row(f'key {name}', 'gui', 'key', results['gnu'][name], results['romacs'][name], rounds,
                             f'{keys} keys x {rounds}; wall pre-command to idle; samples GNU {results["gnu"][name]["samples"]},'
                             f' Romacs {results["romacs"][name]["samples"]}; {romacs_note}'), neomacs(name))
            for name in KEYS]
    if not ctx.quick:
        rows.append(with_neomacs(row('C-x C-f 5,000 files, per key', 'gui', 'key', results['gnu']['C-x C-f'],
                                     results['romacs']['C-x C-f'], rounds,
                                     check_note({e: results[e]['C-x C-f'] for e in ('gnu', 'romacs')},
                                                'fido-vertical, keys of file-1; candidates GNU'
                                                f' {results["gnu"]["C-x C-f"]["candidates"]},'
                                                f' Romacs {results["romacs"]["C-x C-f"]["candidates"]}; {romacs_note}')),
                                 neomacs('C-x C-f')))
        layer = results['romacs']['layer']
        rows.append(row('C-x C-f 5,000 files, per key, romacs-complete', 'gui', 'key', None, layer, rounds,
                        f'Romacs only: its default completion (the layer, fzf), keys of file-1; candidates {layer["candidates"]},'
                        f' minibuffer {layer["check"]!r}; {romacs_note}'))
    return rows


def session_figures(ctx, engine, text, keys, gap, rounds):
    """One ENGINE session's key rows, and for Romacs what it shows of the store."""
    figures = {}
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
        figures = {name: {'instructions': median(p['instructions'] for p in runs),
                          'wall': median(p['wall'] for p in runs),
                          'samples': sum(p['samples'] for p in runs)}
                   for name, runs in phases.items()}
        if not ctx.quick:
            figures['C-x C-f'] = completion(session, ctx.fixtures / 'd5000', rate, rounds, FIDO)
            if engine == 'romacs':
                figures['layer'] = completion(session, ctx.fixtures / 'd5000', rate, rounds, LAYER)
        if engine == 'romacs':
            figures['romacs_note'] = session.romacs_note()
    finally:
        session.stop()
    return figures


# The two completions C-x C-f runs under: GNU's fido-vertical in both
# engines, and Romacs's defaults, its layer with the standard styles
# (fzf first) the comparison session left.
FIDO = '(fido-vertical-mode 1)'
LAYER = ("(progn (custom-reevaluate-setting 'completion-styles)"
         " (custom-reevaluate-setting 'completion-category-overrides) (romacs-complete-mode 1))")


def completion(session, directory, rate, rounds, display):
    session.ask(f'(progn (setq icomplete-compute-delay 0) {display}'
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
                f'first launch: from source, builds the init layer; wall only; {ctx.packages_note()}'),
            row('init body, Romacs layer', 'init', 'run', gnu,
                {'instructions': None, 'wall': median(s[0] for s in bodies['romacs'][1:])}, ctx.runs,
                f'{layer_note(unlayered)}; {ctx.packages_note()}'),
            row('start to init done, layer', 'init', 'run',
                {'instructions': None, 'wall': median(s[1] for s in bodies['gnu'])},
                {'instructions': None, 'wall': median(s[1] for s in bodies['romacs'][1:])}, ctx.runs,
                f'spawn to the init\'s last form; wall only; {ctx.packages_note()}')]
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
        rows += [row('gnosis dashboard open', 'apps', 'run', *steps['open'], 1,
                     f'gnosis_parity_live, DB copy; {ctx.packages_note()}'),
                 row('gnosis back to dashboard', 'apps', 'run', *steps['back'], 1,
                     f'from all themata, q; {ctx.packages_note()}')]
    else:
        ctx.skipped.append('gnosis: no --gnosis-db/--gnosis-db-repo')
    try:
        urllib.request.urlopen(real_init.hermes_url(config) + '/', timeout=3)
    except OSError as error:
        ctx.skipped.append(f'hermes: no dashboard at {real_init.hermes_url(config)} ({error})')
        return rows
    steps = parity_steps(ctx, 'hermes', ['--workflow', 'sessions', '--until', 'open'])
    rows.append(row('hermes dashboard open', 'apps', 'run', *steps['open'], 1,
                    f'hermes_parity_live, read-only; {ctx.packages_note()}'))
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
            for engine in ENGINE_NAMES:
                if r.get(engine) and r[engine].get('wall') is not None:
                    r[engine]['wall_refused'] = r[engine].pop('wall')
                for metric in ('block', 'first', 'key'):
                    if r.get(engine) and r[engine].get(metric) is not None:
                        r[engine][f'{metric}_refused'] = r[engine].pop(metric)
            r['note'] = f'wall refused: load {r["load"]:.1f} > {max_load}; {r["note"]}'
    return rows


def with_ratios(rows, previous):
    """ROWS with Romacs/GNU ratios and changes against PREVIOUS's rows."""
    before = {r['row']: r for r in (previous or {}).get('rows', [])}
    for r in rows:
        for metric in ('instructions', 'wall', 'block'):
            value = (r['romacs'] or {}).get(metric)
            r[f'{metric}_ratio'] = ratio(value, (r['gnu'] or {}).get(metric))
            r[f'{metric}_change'] = change(value, ((before.get(r['row']) or {}).get('romacs') or {}).get(metric))
            if 'gnu_native' in r:
                r[f'native_{metric}_ratio'] = ratio(value, (r['gnu_native'] or {}).get(metric))
            if 'neomacs' in r:
                r[f'neomacs_{metric}_ratio'] = ratio(value, (r['neomacs'] or {}).get(metric))
    return with_medians(rows)


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
             f'{meta["max_load"]}.  Previous: {meta["previous"] or "none"}.  Romacs Lisp: {meta["romacs_lisp"]}.', '',
             '| workload | per | GNU instr | Romacs instr | R/G | Δ instr | GNU wall | Romacs wall | R/G | Δ wall | note |',
             '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|']
    for r in results['rows']:
        if r['group'] in ('elb', 'session'):
            continue
        gnu, romacs = r['gnu'] or {}, r['romacs'] or {}
        lines.append(f'| {r["row"]} | {r["unit"]} | {scaled(gnu.get("instructions"), "instructions")} | '
                     f'{scaled(romacs.get("instructions"), "instructions")} | {times(r["instructions_ratio"])} | '
                     f'{percent(r["instructions_change"])} | {scaled(gnu.get("wall"), "s")} | '
                     f'{scaled(romacs.get("wall"), "s")} | {times(r["wall_ratio"])} | {percent(r["wall_change"])} | '
                     f'{r["note"]} |')
    lines += session_table(results) + elb_table(results) + neomacs_table(results)
    for line in results['skipped']:
        lines.append(f'\nNot run: {line}')
    return '\n'.join(lines) + '\n'


def session_table(results):
    """The session rows, apart: read by wall and block, not instructions."""
    rows = [r for r in results['rows'] if r['group'] == 'session']
    if not rows:
        return []
    lines = ['', 'Sessions: wall from key to result, block the longest time the main VM ran no timer.', '',
             '| row | per | GNU wall | Romacs wall | R/G | GNU block | Romacs block | R/G | R/G instr | note |',
             '|---|---|---:|---:|---:|---:|---:|---:|---:|---|']
    for r in rows:
        gnu, romacs = r['gnu'] or {}, r['romacs'] or {}
        lines.append(f'| {r["row"]} | {r["unit"]} | {scaled(gnu.get("wall"), "s")} | {scaled(romacs.get("wall"), "s")} | '
                     f'{times(r["wall_ratio"])} | {scaled(gnu.get("block"), "s")} | {scaled(romacs.get("block"), "s")} | '
                     f'{times(r["block_ratio"])} | {times(r["instructions_ratio"])} | '
                     f'{"; ".join(filter(None, [readme.session_extras(r), r["note"]]))} |')
    return lines


def neomacs_table(results):
    """neomacs's column, apart: only a run with --neomacs has one."""
    rows = [r for r in results['rows'] if 'neomacs' in r]
    if not rows:
        return []
    build = results['meta']['neomacs']
    lines = ['', f'neomacs {build["version"]}, commit {build["commit"] or "not given"}: as it installs.', '',
             '| workload | neomacs instr | R/N | neomacs wall | R/N |', '|---|---:|---:|---:|---:|']
    for r in rows:
        lines.append(f'| {r["row"]} | {scaled(r["neomacs"].get("instructions"), "instructions")} | '
                     f'{times(r.get("neomacs_instructions_ratio"))} | {scaled(r["neomacs"].get("wall"), "s")} | '
                     f'{times(r.get("neomacs_wall_ratio"))} |')
    return lines


def elb_table(results):
    """The elisp-benchmarks rows, apart: pure Lisp compute, not an editor workload."""
    rows = [r for r in results['rows'] if r['group'] == 'elb']
    if not rows:
        return []
    meta = results['meta']
    native = meta.get('elb_native')
    lines = ['', f'GNU ELPA elisp-benchmarks {ELB[0]} (pure Lisp compute): GNU .elc'
             + (', GNU native-compiled' if native else '; the GNU engine has no native compilation')
             + f', Romacs {meta["romacs_lisp"]}.', '',
             '| member | GNU instr | GNU native instr | Romacs instr | R/G | GNU wall | GNU native wall | Romacs wall '
             '| R/G | R/native | note |', '|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|']
    for r in rows:
        gnu, native_, romacs = r['gnu'] or {}, r.get('gnu_native') or {}, r['romacs'] or {}
        lines.append(f'| {r["row"]} | {scaled(gnu.get("instructions"), "instructions")} | '
                     f'{scaled(native_.get("instructions"), "instructions")} | '
                     f'{scaled(romacs.get("instructions"), "instructions")} | {times(r["instructions_ratio"])} | '
                     f'{scaled(gnu.get("wall"), "s")} | {scaled(native_.get("wall"), "s")} | '
                     f'{scaled(romacs.get("wall"), "s")} | {times(r["wall_ratio"])} | '
                     f'{times(r.get("native_wall_ratio"))} | {r["note"]} |')
    return lines


def previous_results(out_root, explicit, current):
    if explicit:
        return Path(explicit)
    found = sorted((p for p in out_root.glob('*/*/results.json') if p.parent != current),
                   key=lambda p: json.loads(p.read_text())['meta']['started'])
    return found[-1] if found else None


# --- main -----------------------------------------------------------------------

class Context:
    def __init__(self, args, run_dir, cache, fixtures_dir, runtime, romacs, store):
        """STORE: the compiled store ROMACS loads from; None for --romacs-source."""
        self.args, self.run_dir, self.cache, self.fixtures, self.store = args, run_dir, cache, fixtures_dir, store
        self.runs, self.quick, self.tools = args.runs, args.quick, args.tools
        self.engines = {'gnu': args.gnu, 'romacs': str(romacs)}
        self.gnu_build = gnu_build(args.gnu)
        if args.neomacs:
            self.engines['neomacs'] = str(Path(args.neomacs).expanduser())
        self.compared = tuple(self.engines)
        self.memory = {'neomacs': NEOMACS_MEMORY}
        self.engine_env = {'gnu': {}, 'neomacs': {}, 'romacs': {'ROMACS_RUNTIME_ROOT': str(runtime),
                                                 'ROMACS_LISP_DIR': str(runtime / 'gnu-lisp'),
                                                 'ROMACS_COMPILED_LISP': 'on' if store else 'off',
                                                 'ROMACS_GNU_SOURCE': os.environ.get('ROMACS_GNU_SOURCE', '')}}
        # Private trees hold compositor sockets, which take 108 bytes at most.
        (cache.parent / 'w').mkdir(exist_ok=True)
        self.work = Path(tempfile.mkdtemp(dir=cache.parent / 'w'))
        if len(str(self.work)) > 75:  # + /romacs-XXXXXXXX/wayland-NN
            raise SystemExit(f'{self.work}: too long for socket paths; give a shorter --out')
        self.engine_env = {name: self.engine_env[name] for name in self.engines}
        self.skipped = []
        self.elb_native = None
        self.header_bytes = None
        self.history_commits = None

    def attempt(self, engine, function):
        """FUNCTION's result; neomacs, which a row need not have, says why it
        has none.  GNU's and Romacs's failures are the row's."""
        try:
            return function()
        except SESSION_FAILURES as error:
            if engine != 'neomacs':
                raise
            self.log(f'neomacs {time.monotonic_ns()}', traceback.format_exc())
            return {'instructions': None, 'wall': None, 'failed': f'not run: {str(error)[:160]}'}

    def store_marks(self):
        return store_marks(self.store) if self.store else {}

    def store_since(self, marks):
        return store_since(self.store, marks) if self.store else {'compiled': set(), 'published': 0}

    def romacs_note(self, samples):
        return store_note(samples) if self.store else 'Romacs: source'

    def packages_note(self):
        """For a row that loads the harness's private copies of its packages."""
        return ('Romacs: shipped Lisp from the compiled store, these package copies read as source, loads not checked'
                if self.store else 'Romacs: source')

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
    return group not in BY_NAME_ONLY and (quick or not args.quick)


def neomacs_build(args):
    """What --neomacs is: its version, the program it runs and that program's
    SHA-256, and the commit given for it, which it does not report itself."""
    path = Path(args.neomacs).expanduser()
    text = path.read_bytes()
    found = re.search(rb'"(/[^"\s]+\.neomacs-wrapped)"', text) if text.startswith(b'#!') else None
    program = Path(found[1].decode()) if found else path.resolve()
    version = subprocess.run([str(path), '--version'], capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
    return {'path': str(path), 'program': str(program), 'sha256': hashlib.sha256(program.read_bytes()).hexdigest(),
            'version': version.stdout.split('\n', 1)[0].strip(), 'commit': args.neomacs_commit}


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
    parser.add_argument('--gnu', default='/usr/bin/emacs', help='the GNU engine (default: this machine\'s installation)')
    parser.add_argument('--runtime')
    parser.add_argument('--commit')
    parser.add_argument('--quick', action='store_true')
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--only', nargs='+', help='groups, or workloads (a batch row, micro, ERT 25 files, elb,'
                        ' session, launch, keys, init, apps)')
    parser.add_argument('--max-load', type=float, default=8.0)
    parser.add_argument('--out', default=os.environ.get('ROMACS_PERFBENCH_OUT', '~/.cache/romacs/perf-bench'))
    parser.add_argument('--previous', help='results.json to compare with (default: the newest saved run)')
    parser.add_argument('--gnosis-db')
    parser.add_argument('--gnosis-db-repo')
    parser.add_argument('--profile-config', default=os.environ.get(real_init.CONFIG_ENV),
                        help='the real-init profile config (real_init.py)')
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--keep', action='store_true', help='keep the private trees')
    parser.add_argument('--neomacs', help='add neomacs as a third engine, as this program installs')
    parser.add_argument('--neomacs-commit', help='the commit --neomacs was built from, recorded with its build')
    parser.add_argument('--romacs-source', action='store_true',
                        help='measure Romacs from source, with no compiled store or world, not as an installation runs')
    args = parser.parse_args()
    if not os.environ.get('ROMACS_GNU_SOURCE'):
        raise SystemExit('needs ROMACS_GNU_SOURCE, the Emacs tree the fixtures come from')
    os.environ.pop('LD_LIBRARY_PATH', None)  # a toolchain's would break an installed engine
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    runtime, commit = runtime_of(args), commit_of(args)
    parity_live.GNU = args.gnu
    out_root = Path(args.out).expanduser().resolve()
    cache = out_root / 'cache'
    cache.mkdir(parents=True, exist_ok=True)
    romacs, store, install = Path(args.romacs).resolve(), None, None
    if not args.romacs_source:
        romacs, install = stage_install(cache, romacs, runtime)
        runtime, store = romacs.parent, romacs.with_name(f'{romacs.name}.lisp')
    os.environ.update(ROMACS_RUNTIME_ROOT=str(runtime), ROMACS_LISP_DIR=str(runtime / 'gnu-lisp'),
                      ROMACS_COMPILED_LISP='on' if store else 'off')
    started = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())
    run_dir = out_root / commit[:12] / started
    run_dir.mkdir(parents=True)
    fixtures_dir, pins = fixtures(cache, os.environ['ROMACS_GNU_SOURCE'])
    ctx = Context(args, run_dir, cache, fixtures_dir, runtime, romacs, store)
    meta = {'commit': commit, 'started': started, 'romacs': str(Path(args.romacs).resolve()),
            'romacs_lisp': 'compiled store' if store else 'source', 'install': install,
            'neomacs': neomacs_build(args) if args.neomacs else None,
            'romacs_sha256': hashlib.sha256(Path(args.romacs).read_bytes()).hexdigest(), 'runtime': str(runtime),
            'gnu': args.gnu, 'gnu_build': ctx.gnu_build, 'runs': args.runs, 'quick': args.quick, 'max_load': args.max_load,
            'load_start': load(), 'fixtures': pins, 'cpus': os.cpu_count()}
    print(f'perfbench {commit[:12]} -> {run_dir}; load {meta["load_start"]:.1f}', flush=True)
    rows = []
    try:
        batch = Batch(ctx)
        plan = [(name, group, quick, lambda w=workload, n=name: [batch.workload(n, w)])
                for name, group, workload, quick in BATCH]
        plan += [('micro', 'micro', True, lambda: micro_rows(ctx, batch)),
                 ('ERT 25 files', 'ert', False, lambda: [ert_row(ctx, batch)]),
                 ('elb', 'elb', False, lambda: elb_rows(ctx, batch)),
                 ('session', 'session', False, lambda: session_rows(ctx)),
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
                r.setdefault('load', watch.peak)
            rows += made
            print(f'  {name}: {time.time() - started_at:.0f}s, load up to {watch.peak:.1f}', flush=True)
    finally:
        if not args.keep:
            shutil.rmtree(ctx.work, ignore_errors=True)
    meta['load_end'] = load()
    meta['elb_native'] = ctx.elb_native
    meta['header_bytes'] = ctx.header_bytes
    meta['history_commits'] = ctx.history_commits
    meta['history_commit'] = UPSTREAM
    previous = previous_results(out_root, args.previous, run_dir)
    meta['previous'] = str(previous) if previous else None
    results = {'meta': meta, 'skipped': ctx.skipped,
               'rows': with_ratios(gate_walls(rows, args.max_load), json.loads(previous.read_text()) if previous else None)}
    (run_dir / 'results.json').write_text(json.dumps(results, indent=1) + '\n')
    text = table(results)
    (run_dir / 'table.md').write_text(text)
    print(text)
    # What README's lead needs comes from this run or the run has failed.
    if selected(args, 'session', 'session', False):
        missing = readme.lead_missing(results)
        if missing:
            print(f'README lead: no value for {", ".join(missing)}', file=sys.stderr)
            return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
