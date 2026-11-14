#!/usr/bin/env python3
"""Standing benchmark suite: Romacs against pinned GNU Emacs, row by row.

Each row runs one workload in pinned GNU Emacs ($ROMACS_GNU_ORACLE, or --gnu)
and in one Romacs build, each engine in private HOME/XDG/TMPDIR trees, on the
same pinned inputs.  Its main figure is user-space instructions (`perf stat -e
instructions:u', every thread of the process), with wall time beside it, the
median of --runs runs, and for each the Romacs/GNU ratio and the change of
Romacs's figure since the previous saved run.  Results go to OUT/<commit>/<UTC
time>/ as results.json and table.md; the table is printed.  The whole suite
takes under 15 minutes on a quiet machine, once the build is staged.

Rows are picked by area or name: `--only syntax,typing' or `--only "key
self-insert text"'; a name may be a glob (`--only "*c-ts-mode"').  `--list'
prints every row and its areas.  Without --only, every row but the whole
elisp-benchmarks suite runs.

Areas:
  startup     -Q --batch to exit; GUI launch to first paint (Romacs only) and
              to the first idle moment after startup.
  typing      per key in a GUI session: self-insert, C-f and C-n in text;
              self-insert in the middle of xdisp.c (c-ts-mode) and subr.el;
              and the syntax rows' key in the middle.
  scroll      per key in a GUI session: C-v down xdisp.c (c-ts-mode) and
              subr.el, each screen fontified as it comes.
  find-file   C-x C-f over 5,000 files in fido-vertical, and in Romacs's own
              completion; xdisp.c and a generated 15 MB header opened in
              c-ts-mode to the window's final faces; dired of the 5,000 files.
  completion  the C-x C-f rows, and M-x in fido-vertical.
  font-lock   per keystroke and per buffer: the typing and scroll rows in code,
              the syntax rows' key and font-lock-ensure, and Org's ORG-NEWS.
  syntax      syntax-ppss after an edit, scan-lists, forward-comment, a key in
              the middle, font-lock-ensure: subr.el in emacs-lisp-mode, fileio.c
              and xdisp.c in c-mode and c-ts-mode (the syntax-ppss lane's rig).
              xdisp.c's c-mode font-lock-ensure is left out: fileio.c's says
              the same at a fraction of the time.
  lisp        the call path (named and closure calls, mapcar, dolist + funcall),
              uncompiled Lisp, core primitives, and three members of GNU ELPA's
              elisp-benchmarks.
  search      occur over xdisp.c, project-find-regexp over the Emacs tree.
  vc          vc-print-root-log over upstream master's whole history.
  theme       a theme switch, cycled and on first visit.
  commands    describe-function, 20 shell-command-to-string.
  elb         by name only: every member of elisp-benchmarks.

How rows count:
  batch       `-Q --batch -l perfbench.el', one process per run: instructions
              are the run's less a run doing only its setup, wall the body's
              own float-time: whatever threads a tree-sitter parser runs on
              count.  Each engine's result (match
              counts, positions) is compared, and a difference is noted.
  lisp        a loop of 1,000,000 iterations compiled in both engines (GNU
              byte-compiled, Romacs from its compiled store), per iteration
              beyond an empty loop; `uncompiled' rows read source in both, GNU
              into its interpreter, per whole iteration less a run that only
              loads the file.  elisp-benchmarks
              members: GNU .elc and natively compiled, Romacs from its store.
  per key     a GUI session on headless labwc per engine: instructions over
              the keys less the idle rate, every thread but llvmpipe's (Mesa's
              software GL stands in for the GPU there); wall from
              `pre-command-hook' to the next idle moment, after redisplay.
              Romacs takes GNU's completion here (fido-vertical, the styles
              without fzf), but in the romacs-complete row.
  session     the shipped-defaults rows (find-file, the history log, themes):
              wall from the key to the result, block the longest gap between
              ticks of a 5 ms timer, which cannot run while the main VM is
              busy.  Read by wall and block: a worker VM's path does more work
              in total to keep the main VM free.

Romacs runs as an installation runs it.  Before anything is measured its
binary is staged beside its Lisp trees and those are compiled ahead of use
(`--compile-lisp-tree', as an installer does), cached per binary and runtime.
Batch rows set ROMACS_COMPILED_LISP=on and boot from a batch world prepared
first.  A workload's own file loads by name with an `.elc' beside it, as a
package installs.  Each row's note counts the loads the store's use log
confirms and names any read as source.  --romacs-source measures with no store
or world.  --neomacs BIN adds a third engine, its own column.

Wall times are refused (instructions still reported) for a row during which
the 1-minute load average exceeded --max-load.  Inputs are pinned: the Emacs
tree is a --shared clone of $ROMACS_GNU_SOURCE at PIN, its files checked
against SHA-256s; the 5,000 files are generated and checked by manifest.

usage: perfbench.py --romacs BIN [--runtime DIR] [--commit SHA] [--only AREA|ROW ...] [--list]
                    [--runs 3] [--max-load 8] [--out ~/.cache/romacs/perf-bench (ROMACS_PERFBENCH_OUT)]
                    [--previous RESULTS.json] [--romacs-source] [--neomacs BIN [--neomacs-commit SHA]]
A binary installed beside its runtime (lisp/gnu/, lisp/romacs/) uses that
runtime; otherwise give --runtime.
"""
import argparse
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import re
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

# (row, batch workload, areas)
BATCH = [('startup -Q --batch', 'startup', ('startup',)),
         ('dired 5000 files', 'dired-5000', ('find-file',)),
         ('occur xdisp.c', 'occur-xdisp', ('search', 'xdisp.c')),
         ('project-find-regexp repo', 'project-find-regexp', ('search',)),
         ('describe-function find-file', 'describe-function', ('commands',)),
         ('shell-command 20 runs', 'shell-command', ('commands',)),
         ('font-lock-ensure ORG-NEWS', 'font-lock-org-news', ('font-lock', 'org'))]
# The syntax-ppss lane's rig: (file, its path in the tree, mode); each
# operation's row is named `OPERATION FILE MODE'.
SYNTAX_FILES = [('subr.el', 'lisp/subr.el', 'emacs-lisp-mode'),
                ('fileio.c', 'src/fileio.c', 'c-mode'), ('xdisp.c', 'src/xdisp.c', 'c-mode'),
                ('fileio.c', 'src/fileio.c', 'c-ts-mode'), ('xdisp.c', 'src/xdisp.c', 'c-ts-mode')]
# (operation, row prefix, areas, what it does); perfbench.el's `perfbench--syntax'.
SYNTAX_OPS = [('syntax-ppss', 'syntax-ppss', ('syntax',), '10 edits near the start, each then (syntax-ppss (point-max))'),
              ('scan-lists', 'scan-lists', ('syntax',), '500 times out of the top-level form around the middle, both ways'),
              ('forward-comment', 'forward-comment', ('syntax',), "1000 times over the file's leading comments and back"),
              ('key', 'key in the middle', ('syntax', 'typing', 'font-lock'),
               '20 self-inserted keys, each line refontified and its paren shown'),
              ('font-lock-ensure', 'font-lock-ensure', ('syntax', 'font-lock'), 'the whole buffer, the mode just turned on')]
# Whose time says nothing a cheaper row does not: fileio.c's c-mode font-lock-ensure.
SYNTAX_LEFT_OUT = {('font-lock-ensure', 'xdisp.c', 'c-mode')}


def syntax_rows():
    """(row, file, path, mode, operation, areas, what) of every syntax row."""
    return [(f'{prefix} {name} {mode}', name, path, mode, op, (*areas, mode, name), what)
            for name, path, mode in SYNTAX_FILES for op, prefix, areas, what in SYNTAX_OPS
            if (op, name, mode) not in SYNTAX_LEFT_OUT]


MICRO_N = 1_000_000
# hof-inline's closure-micro file (variants.py), its loop called by the driver,
# with the data the primitives' rows use.
MICRO_HEAD = (';;; -*- lexical-binding: t -*-\n(require (quote cl-lib))\n(defun m-id (x) x)\n'
              '(defun m-add3 (x a b) (+ x a b))\n(defvar m-v (vector 1 2 3 4 5 6 7 8 9 10))\n'
              '(defvar m-fr (lambda (&rest xs) xs))\n(defvar m-s (list "a" "b" "c" "d" "e" "f" "g" "h" "i" "j"))\n'
              '(defvar m-h (let ((h (make-hash-table))) (dotimes (k 1000) (puthash k k h)) h))\n'
              "(defvar m-al (mapcar (lambda (k) (cons k k)) '(a b c d e f g h i j)))\n"
              "(defvar m-pl '(:a 1 :b 2 :c 3 :d 4 :e 5 :f 6 :g 7 :h 8 :i 9 :j 10))\n"
              '(defun m-run (n) (let ((i 0) (a 1) (b 2) (c 3) (d 4) (s 0) (l (list 1 2 3 4 5 6 7 8 9 10))'
              ' (f0 (lambda (x) x)) (f1 nil) (f2 nil) (f4 nil)) (setq f1 (let ((a a)) (lambda (x) (+ x a))))'
              ' (setq f2 (let ((a a) (b b)) (lambda (x) (+ x a b)))) (setq f4 (let ((a a) (b b) (c c) (d d))'
              ' (lambda (x) (+ x a b c d)))) (list a b c d s l f0 f1 f2 f4) (while (< i n) BODY (setq i (1+ i))) s))\n')
# (row, loop body); `micro loop' is what every row's figures are beyond, and no row.
MICRO_LOOP = 'nil'
MICRO = [('micro named call', '(m-id i)'),
         ('micro closure call', '(funcall f0 i)'),
         ('micro mapcar 10', '(mapcar f0 l)'),
         ('micro dolist + funcall 10', '(dolist (x l) (funcall f0 x))'),
         ('prim gethash', '(gethash (% i 1000) m-h)'),
         ('prim puthash', '(puthash (% i 1000) i m-h)'),
         ('prim assq', "(assq 'j m-al)"),
         ('prim plist-get', '(plist-get m-pl :j)'),
         ('prim format', '(format "%d-%s" i "x")'),
         ('prim concat', '(concat (car m-s) "_row")'),
         ('prim string-match', '(string-match "_\\\\([a-z]+\\\\)_" "glyph_row_struct")'),
         ('prim intern', '(intern "glyph_row")'),
         ('prim insert + delete-char', '(let ((buffer-undo-list t)) (insert "x") (delete-char -1))')]
# Lisp no one compiled, as an init file's top level or M-: runs: GNU reads the
# source into its interpreter; Romacs runs it as it ships.  Whole iterations,
# the loop included, since that loop is itself what is being interpreted.
UNCOMPILED = [('uncompiled arithmetic', '(setq s (+ s (% i 97)))'),
              ('uncompiled named call', '(m-id i)'),
              ('uncompiled dolist 10', '(dolist (x l) (setq s (+ s x)))')]
# GNU ELPA's elisp-benchmarks, the suite GNU evaluates native compilation
# with: (version, tarball URL, its SHA-256, member files).
ELB = ('1.16', 'https://elpa.gnu.org/packages/elisp-benchmarks-1.16.tar',
       'e222500d387152bdf3d5a178681071ddf4d9921adf73f97adc3942f3135aad6c', 17)
# The members the lisp area runs: a parser, objects and byte packing, each a
# few seconds; `--only elb' runs every member.
ELB_DEFAULT = ('smie', 'eieio', 'pack-unpack')
# (column, engine, how it loads the member files); gnu_native only when
# the GNU engine has native compilation, neomacs only with --neomacs.
ELB_COLUMNS = (('gnu', 'gnu', 'elc'), ('gnu_native', 'gnu', 'eln'), ('romacs', 'romacs', 'store'),
               ('neomacs', 'neomacs', 'elc'))
# An error a run reported instead of signalling: the byte compiler's, or
# the suite's demoted load and run errors.
ELB_ERROR = re.compile(r'(?:^|: )(Error(?: loading| running)?: .*)', re.M)
SESSION_LISP = HERE / 'perfbench-session.el'
SESSION_METRICS = ('wall', 'block', 'instructions', 'first', 'key', 'keys', 'cores')
# The comparison session's per-key rows: (row, areas).  Text rows type into a
# 200-line text; code rows visit their file afresh each round, C in c-ts-mode,
# and type at its middle or scroll from its top.
LAUNCH = [('launch to first paint', ('startup',)), ('launch to idle', ('startup',))]
TEXT_KEYS = {'key self-insert text': ['a'], 'key C-f text': ['-M', 'ctrl', 'f', '-m', 'ctrl'],
             'key C-n text': ['-M', 'ctrl', 'n', '-m', 'ctrl']}
CTRL = lambda keys: ['-M', 'ctrl', *[a for k in keys for a in ('-k', k)], '-m', 'ctrl']  # noqa: E731
# (row, file in the tree, c-ts-mode, where point starts, the key, seconds between keys, areas)
CODE_KEYS = [('key self-insert xdisp.c c-ts-mode', 'src/xdisp.c', True, 'middle', ['x'], .15,
              ('typing', 'font-lock', 'c-ts-mode', 'xdisp.c')),
             ('key self-insert subr.el emacs-lisp-mode', 'lisp/subr.el', False, 'middle', ['x'], .15,
              ('typing', 'font-lock', 'emacs-lisp-mode', 'subr.el')),
             ('scroll C-v xdisp.c c-ts-mode', 'src/xdisp.c', True, 'top', CTRL(['v']), .2,
              ('scroll', 'font-lock', 'c-ts-mode', 'xdisp.c')),
             ('scroll C-v subr.el emacs-lisp-mode', 'lisp/subr.el', False, 'top', CTRL(['v']), .2,
              ('scroll', 'font-lock', 'emacs-lisp-mode', 'subr.el'))]
# (row, the keys that open the prompt, the keys typed into it, completion, areas)
KEYSYM = {'-': 'minus'}
COMPLETIONS = [('C-x C-f 5000 files fido', CTRL(['x', 'f']), 'file-1', 'fido', ('find-file', 'completion')),
               ('M-x fido', ['-M', 'alt', 'x', '-m', 'alt'], 'find-fi', 'fido', ('completion',)),
               ('C-x C-f 5000 files romacs-complete', CTRL(['x', 'f']), 'file-1', 'layer', ('find-file', 'completion'))]
KEYS_PER_ROUND = 20
# The shipped-defaults session's rows: (row, command in perfbench-session.el,
# rounds (None: --runs), seconds a round may take, areas).  First visits come
# before the cycled rows load their themes.
SESSION_SHIPPED = [('open xdisp.c c-ts-mode', 'open-xdisp-ts', None, 120, ('find-file', 'font-lock', 'c-ts-mode', 'xdisp.c')),
                   ('open 15 MB header c-ts-mode', 'open-header-ts', None, 300, ('find-file', 'c-ts-mode')),
                   ('vc-print-root-log whole history', 'vc-history', 1, 300, ('vc',)),
                   ('theme switch first visit', 'theme-first', None, 120, ('theme',)),
                   ('theme switch cycled', 'theme-cycle', None, 120, ('theme',))]
# Each engine's own shipped themes: (to cycle through, to visit once each).
# Romacs ships none of GNU's, whose modus and tango it replaced with its ports.
GNU_THEMES = (['modus-operandi', 'modus-vivendi', 'tango-dark'], ['wombat', 'leuven', 'adwaita', 'deeper-blue', 'misterioso'])
SESSION_THEMES = {'gnu': GNU_THEMES, 'neomacs': GNU_THEMES,
                  'romacs': (['dracula', 'nord', 'gruvbox-dark'], ['zenburn', 'solarized-dark', 'tokyonight-night',
                                                                   'monokai-classic', 'catppuccin-mocha'])}
HEADER_LINES = 146_000  # about 15 MB, the shape of a generated register header
# Address space for a neomacs batch run, some fifty times what one needs.
# Romacs reserves a large arena and is not limited this way.
NEOMACS_MEMORY = 8 << 30
INSTALL_TREES = ('lisp', 'etc')  # what an installation ships beside its binary
LISP_JOBS = 8  # compile processes at a time, staging an installation


def catalogue():
    """(row, areas) of every row, in table order; the elb area's members
    beyond ELB_DEFAULT are found by loading the suite, so they are not here."""
    rows = [(name, areas) for name, _, areas in BATCH]
    rows += [(name, areas) for name, *_, areas, _ in syntax_rows()]
    rows += [(name, ('lisp',)) for name, _ in MICRO + UNCOMPILED]
    rows += [(f'elb {member}', ('lisp', 'elb')) for member in ELB_DEFAULT]
    rows += LAUNCH + [(name, ('typing',)) for name in TEXT_KEYS]
    rows += [(name, areas) for name, *_, areas in CODE_KEYS]
    rows += [(name, areas) for name, *_, areas in COMPLETIONS]
    rows += [(name, areas) for name, *_, areas in SESSION_SHIPPED]
    return rows


def selection(only, rows):
    """The rows ONLY (a list of comma-separated areas, row names and globs)
    selects from ROWS, catalogue()'s, and whether it asks for every elb
    member.  None selects every row.  A word that matches nothing stops the run."""
    if not only:
        return [name for name, _ in rows], False
    words = [word.strip() for item in only for word in item.split(',') if word.strip()]
    areas = {area for _, row_areas in rows for area in row_areas}
    unknown = [w for w in words if w not in areas and not any(fnmatch.fnmatchcase(name, w) for name, _ in rows)
               and w != 'elb' and not w.startswith('elb ')]
    if unknown:
        raise SystemExit(f'--only: nothing is called {unknown}; areas: {", ".join(sorted(areas))} (--list names the rows)')
    chosen = [name for name, row_areas in rows
              if any(w in row_areas or fnmatch.fnmatchcase(name, w) for w in words)]
    members = [w for w in words if w.startswith('elb ') and w not in chosen and not any(c in w for c in '*?[')]
    return chosen + members, 'elb' in words


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
                               'ROMACS_RUNTIME_ROOT': str(root), 'ROMACS_LISP_DIR': str(root / 'lisp/gnu')},
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
        self.setups, self.warmed = {}, set()
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

    def figures(self, engine, workload, extra=None, setup=None):
        """ENGINE's figures for WORKLOAD with EXTRA's environment.  SETUP names
        what its setup does, shared by every row with that name: its
        setup-only runs are made once.  So is Romacs's first use of a workload
        in a mode, whose compiling is no part of a warm row (its note says so)."""
        runs, setup = self.ctx.runs, setup or workload
        first, family = {}, (workload, (extra or {}).get('PERFBENCH_MODE'))
        if engine == 'romacs' and self.ctx.store and family not in self.warmed:
            self.warmed.add(family)
            first = self.once(engine, workload, extra=extra)
        full = [self.once(engine, workload, extra=extra) for _ in range(runs)]
        if workload == 'startup':
            return {'instructions': median(r['instructions'] for r in full),
                    'wall': median(r['process_wall'] for r in full), 'check': None, 'samples': full,
                    'first_use': first.get('published', 0)}
        if (engine, setup) not in self.setups:
            self.setups[engine, setup] = median(self.once(engine, workload, skip=True, extra=extra)['instructions']
                                                for _ in range(runs))
        skipped = self.setups[engine, setup]
        return {'instructions': median(r['instructions'] for r in full) - skipped,
                'wall': median(r['seconds'] for r in full), 'check': full[0]['check'],
                'setup_instructions': skipped, 'samples': full, 'first_use': first.get('published', 0)}

    def workload(self, name, workload, extra=None, setup=None, group='batch', what=''):
        engines = {e: self.ctx.attempt(e, lambda e=e: self.figures(e, workload, extra, setup)) for e in self.ctx.compared}
        samples = {e: figures.pop('samples', None) for e, figures in engines.items()}
        note = 'process to exit' if workload == 'startup' else '; '.join(filter(None, [what, 'less its setup']))
        for engine, figures in engines.items():
            if figures.get('instructions') is not None and figures['instructions'] <= 0:
                note += f'; {ENGINE_NAMES[engine]}: instructions not above the setup run\'s ({figures["instructions"]:+.0f})'
                figures['instructions'] = None
        if engines['romacs'].get('first_use'):
            note += f'; Romacs: {engines["romacs"]["first_use"]} artifacts compiled on first use, before these runs'
        made = row(name, group, 'run', engines['gnu'], engines['romacs'], self.ctx.runs,
                   f'{check_note(engines, note)}; {self.ctx.romacs_note(samples["romacs"])}')
        return with_neomacs(made, engines)


def syntax_row(batch, name, path, mode, op, what):
    """One syntax operation's row: operations that start warm share a setup."""
    extra = {'PERFBENCH_FILE': f'{TREE}/{path}', 'PERFBENCH_MODE': mode, 'PERFBENCH_OP': op}
    setup = f'syntax {path} {mode} {"plain" if op == "font-lock-ensure" else "warm"}'
    return batch.workload(name, 'syntax', extra, setup, 'syntax', what)


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
    """NOTE, saying so when GNU's and Romacs's results differ; a row one of
    them has no figures for compares nothing."""
    if 'gnu' not in engines or 'romacs' not in engines:
        return note
    gnu, romacs = engines['gnu'].get('check'), engines['romacs'].get('check')
    if gnu != romacs:
        note = f'{note}; result differs: GNU {gnu!r}, Romacs {romacs!r}'.lstrip('; ')
    return note


def micro_env(ctx, engine, source):
    """How ENGINE loads the micro loop SOURCE, as it loads a package: GNU and
    neomacs the .elc their own compilers made beside it, Romacs by name from
    its store."""
    loaded = source.with_suffix('') if engine == 'romacs' and ctx.store else source
    return {'PERFBENCH_MICRO': str(loaded), 'PERFBENCH_N': str(MICRO_N)}


def micro_runs(ctx, batch, engine, source):
    """--runs runs of the micro loop SOURCE in ENGINE, Romacs's after a run
    whose compiling is not counted."""
    extra = micro_env(ctx, engine, source)
    if engine == 'romacs' and ctx.store:
        batch.once('romacs', 'micro', skip=True, extra=extra)
    return [batch.once(engine, 'micro', extra=extra) for _ in range(ctx.runs)]


def micro_setup(ctx, batch, engine, source):
    """The median instructions of --runs runs that only load SOURCE."""
    extra = micro_env(ctx, engine, source)
    return median(batch.once(engine, 'micro', skip=True, extra=extra)['instructions'] for _ in range(ctx.runs))


def micro_source(directory, body):
    """The micro file of loop BODY in DIRECTORY, named by its text."""
    text = MICRO_HEAD.replace('BODY', body)
    source = directory / f'{hashlib.sha256(text.encode()).hexdigest()[:12]}.el'
    source.write_text(text)
    return source


def micro_rows(ctx, batch, wanted):
    """The WANTED call-path and primitive rows: per iteration, beyond the
    empty loop, compiled in every engine."""
    selected = [(name, body) for name, body in MICRO if name in wanted]
    if not selected:
        return []
    directory = ctx.cache / 'micro' / ctx.gnu_build['sha256'][:12]  # bytecode is the engine's own
    directory.mkdir(parents=True, exist_ok=True)
    samples = {}
    for name, body in [('loop', MICRO_LOOP), *selected]:
        source = micro_source(directory, body)
        if not source.with_suffix('.elc').exists():
            run([ctx.engines['gnu'], '-Q', '--batch', '--eval', '(setq byte-compile-warnings nil)',
                 '-f', 'batch-byte-compile', str(source)], batch.env['gnu'], directory, 300)
        compiled = {'gnu': lambda: source.with_suffix('.elc'), 'romacs': lambda: source,
                    'neomacs': lambda: neomacs_compiled(ctx, batch, source)}
        samples[name] = {e: ctx.attempt(e, lambda e=e: micro_runs(ctx, batch, e, compiled[e]())) for e in ctx.compared}

    def per_iteration(name, engine, key):
        return (median(s[key] for s in samples[name][engine])
                - median(s[key] for s in samples['loop'][engine])) / MICRO_N

    def figures(name, engine):
        failed = next((samples[n][engine] for n in ('loop', name) if isinstance(samples[n][engine], dict)), None)
        return failed or {'instructions': per_iteration(name, engine, 'instructions'),
                          'wall': per_iteration(name, engine, 'seconds'), 'check': None}
    rows = []
    for name, body in selected:
        engines = {engine: figures(name, engine) for engine in ctx.compared}
        rows.append(with_neomacs(row(name, 'micro', 'iteration', engines['gnu'], engines['romacs'], ctx.runs,
                                     f'{body}; GNU compiled; {ctx.romacs_note(samples[name]["romacs"])}'), engines))
    return rows


def uncompiled_rows(ctx, batch, wanted):
    """Each WANTED UNCOMPILED row, per whole iteration less a run that only
    loads the file, GNU from source in its interpreter."""
    directory = ctx.cache / 'uncompiled'
    directory.mkdir(parents=True, exist_ok=True)
    rows = []
    for name, body in UNCOMPILED:
        if name not in wanted:
            continue
        source = micro_source(directory, body)
        loaded = {'gnu': source, 'romacs': source, 'neomacs': source}
        samples = {engine: ctx.attempt(engine, lambda e=engine: (micro_runs(ctx, batch, e, loaded[e]), micro_setup(ctx, batch, e, loaded[e])))
                   for engine in ctx.compared}

        def figures(engine):
            taken = samples[engine]
            if isinstance(taken, dict):
                return taken
            runs, setup = taken
            return {'instructions': (median(s['instructions'] for s in runs) - setup) / MICRO_N,
                    'wall': median(s['seconds'] for s in runs) / MICRO_N, 'check': None}
        engines = {engine: figures(engine) for engine in ctx.compared}
        rows.append(with_neomacs(row(name, 'micro', 'iteration', engines['gnu'], engines['romacs'], ctx.runs,
                                     f'{body}; GNU interpreted; less a run that only loads it; '
                                     f'{ctx.romacs_note(samples["romacs"][0])}'), engines))
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


def elb_package(ctx, column, tarball):
    """COLUMN's copy of the suite, kept between runs beside its compiled files
    (each engine compiles what it lacks when it prepares the suite): per GNU
    build, as GNU's .elc and .eln files are its own."""
    version, count = ELB[0], ELB[3]
    root = ctx.cache / 'elb' / ctx.gnu_build['sha256'][:12] / column
    package = root / f'elisp-benchmarks-{version}'
    if not package.exists():
        root.mkdir(parents=True, exist_ok=True)
        subprocess.run(['tar', '-xf', str(tarball), '-C', str(root)], check=True, timeout=60)
    found = len(list((package / 'benchmarks').glob('*.el')))
    if found != count:
        raise Refused(f'elisp-benchmarks {version} has {found} member files, not {count}')
    return root, package


def elb_rows(ctx, batch, members=None):
    """GNU ELPA's elisp-benchmarks, MEMBERS of it (None: every entry point the
    suite defines in either engine): GNU byte-compiled (and native-compiled
    when it can), Romacs from its store.  Instructions are a run's less a
    setup-only run."""
    tarball = elb_tarball(ctx.cache)
    ctx.elb_native = gnu_native_comp(ctx, batch)
    columns = [c for c in ELB_COLUMNS if c[1] in ctx.engines and (c[0] != 'gnu_native' or ctx.elb_native)]
    prepared = {}
    for column, engine, mode in columns:
        root, package = elb_package(ctx, column, tarball)
        if mode == 'store' and not ctx.store:
            mode = 'source'
        if mode == 'store':  # the .elc an installed package has beside each file
            gnu = elb_package(ctx, 'gnu', tarball)[1]
            for compiled in gnu.rglob('*.elc'):
                if not (package / compiled.relative_to(gnu)).exists():
                    shutil.copy(compiled, package / compiled.relative_to(gnu))
        env = {'PERFBENCH_ELB_DIR': str(package), 'PERFBENCH_ELB_MODE': mode, 'PERFBENCH_ELN': str(root / 'eln')}
        prepared[column] = elb_column(batch, engine, env, ctx.runs)
    found = sorted({e for p in prepared.values() for e in p['entries']})
    if not found:
        raise Refused('; '.join(f'{ENGINE_NAMES[c]}: {p["failed"]}' for c, p in prepared.items()))
    rows = []
    for member in found if members is None else sorted(members):
        watch = LoadWatch()
        samples = watch.during(lambda: {c: elb_samples(ctx, batch, prepared[c], member) for c in prepared})
        engines = {c: elb_member(samples[c], prepared[c].get('setup')) for c in prepared}
        ran = samples['romacs'] if isinstance(samples['romacs'], list) else None
        made = row(f'elb {member}', 'elb', 'run', engines['gnu'], engines['romacs'], ctx.runs,
                   '; '.join(filter(None, [elb_note(engines), ran and ctx.romacs_note(ran)])))
        made.update({c: engines[c] for c in ('gnu_native', 'neomacs') if c in engines})
        made['load'] = watch.peak
        rows.append(made)
    summaries = [elb_geomean_row(rows, list(prepared), ctx.runs), elb_median_row(rows, ctx.runs)] if members is None else []
    return rows + summaries


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


def launch_rows(ctx, wanted):
    """Warm launches of each engine: to Romacs's first paint and to idle,
    Romacs's alone when only its first paint is wanted."""
    engines = ctx.compared if 'launch to idle' in wanted else ('romacs',)
    launches = {engine: ctx.attempt(engine, lambda e=engine: launch_samples(ctx, e)) for engine in engines}
    figures = {e: launches[e] if isinstance(launches[e], dict) else
               {'instructions': median(s['instructions'] for s in launches[e]),
                'wall': median(s['idle'] for s in launches[e]), 'check': None} for e in engines}
    rows = []
    if 'launch to first paint' in wanted:
        rows.append(row('launch to first paint', 'gui', 'run', None,
                        {'instructions': None, 'wall': median(s['paint'] for s in launches['romacs'])}, ctx.runs,
                        'Romacs only (its --trace-latency first-paint); GNU has no paint mark; '
                        + ctx.romacs_note(launches['romacs'])))
    if 'launch to idle' in wanted:
        rows.append(with_neomacs(row('launch to idle', 'gui', 'run', figures['gnu'], figures['romacs'], ctx.runs,
                                     'spawn to the first idle after emacs-startup-hook; instructions to exit there; '
                                     + ctx.romacs_note(launches['romacs'])), figures))
    return rows


def session_inputs(ctx, wanted):
    """A private clone of the pinned tree with src/xdisp.c changed, which the
    GUI rows visit and type into; the generated header; and, when the history
    row runs, a clone at upstream master."""
    repo = ctx.work / 'session-repo'
    if not repo.exists():
        git('clone', '-q', '--shared', str(ctx.fixtures / TREE), str(repo))
        with (repo / 'src/xdisp.c').open('a') as out:
            out.write('/* perfbench */\n')
    header = ctx.work / 'generated.h'
    if not header.exists():
        header.write_text(generated_header(HEADER_LINES))
        ctx.header_bytes = header.stat().st_size
    history = ctx.work / 'session-history'
    if 'vc-print-root-log whole history' in wanted and not history.exists():
        git('clone', '-q', '--shared', '--no-checkout', str(ctx.fixtures / TREE), str(history))
        git('-c', 'advice.detachedHead=false', 'checkout', '-q', UPSTREAM, cwd=history)
        ctx.history_commits = int(git('rev-list', '--count', 'HEAD', cwd=history))
    return repo, header, history


def generated_header(lines):
    """A generated C register header of LINES lines, the same on every run."""
    def line(n):
        name = f'#define REG_{n // 12:06}_CNTL__FIELD_{n % 12:02}_MASK'
        return f'//REG_{n // 12:06}_CNTL\n' if n % 12 == 0 else f'{name:<96}0x{(n * 2654435761) % 2 ** 32:08X}L\n'
    return ''.join(line(n) for n in range(lines))


def tree_cpu(pid):
    """CPU seconds so far of process PID and its descendants, reaped children included."""
    tick = os.sysconf('SC_CLK_TCK')
    parents, ticks = {}, {}
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                fields = (entry / 'stat').read_text().rpartition(')')[2].split()
            except OSError:
                continue
            parents[int(entry.name)] = int(fields[1])
            ticks[int(entry.name)] = sum(int(f) for f in fields[11:15])

    def within(candidate):
        while candidate in parents:
            if candidate == pid:
                return True
            candidate = parents[candidate]
        return False
    return sum(t for p, t in ticks.items() if within(p)) / tick


def session_command(session, name, rate, rounds, timeout=120):
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
        cpu = tree_cpu(session.engine.process.pid)
        instructions, elapsed = session.counted(act, timeout)
        cpu = tree_cpu(session.engine.process.pid) - cpu
        seconds, block, first, key, keys = parity_live.read_sexp(done.read_text())
        samples.append({'wall': seconds, 'block': block, 'first': first, 'key': key, 'keys': keys,
                        'cores': cpu / elapsed if elapsed else None,
                        'instructions': None if instructions is None else instructions - rate * elapsed})
        check, detail = session.ask(f'(perfbench-session-finish (quote {name}))', 120)
        checks.append(check)
        time.sleep(.5)
    return session_medians(samples, checks) | {'detail': detail}


def session_medians(samples, checks):
    """SAMPLES' medians, when every round made the same thing, or a refusal:
    a round that did something else is no sample of this row."""
    if len(set(map(str, checks))) > 1:
        raise Refused(f'rounds differ: {checks}')
    return {metric: median(s.get(metric) for s in samples) for metric in SESSION_METRICS} | {'check': checks[0]}


def shipped_figures(ctx, engine, inputs, wanted):
    """ENGINE's WANTED shipped-defaults rows, in one session as it ships: a
    row that fails says so and costs that row alone."""
    repo, header, history = inputs
    cycle, visits = SESSION_THEMES[engine]
    symbols = lambda names: f"'({' '.join(names)})"  # noqa: E731
    figures = {}
    session = Session(ctx, engine, shipped=True)
    try:
        session.ask(f'(progn (load {parity_live.lisp_string(str(SESSION_LISP))} nil t t)'
                    f' (setq perfbench-session-repo {parity_live.lisp_string(str(repo))}'
                    f' perfbench-session-header {parity_live.lisp_string(str(header))}'
                    f' perfbench-session-history {parity_live.lisp_string(str(history))}'
                    f' perfbench-session-cycle {symbols(cycle)} perfbench-session-visits {symbols(visits)})'
                    f' {QUIET} t)')
        time.sleep(2)
        rate = session.idle_rate()
        for name, command, rounds, timeout, _ in SESSION_SHIPPED:
            if name not in wanted:
                continue
            # A first visit happens once per theme, however many --runs there are.
            rounds = min(ctx.runs, len(visits)) if command == 'theme-first' else rounds or ctx.runs
            figures[name] = session_row(session, name, lambda c=command, r=rounds, t=timeout:
                                        session_command(session, c, rate, r, t),
                                        f'(perfbench-session-abort (quote {command}))')
        if engine == 'romacs' and not session.stopped:
            figures['romacs_note'] = session.romacs_note()
    finally:
        session.stop()
    return figures


def session_row(session, name, function, recover):
    """FUNCTION's figures, or why it has none, the session left ready for the
    next by RECOVER, a form.  A session that stops answering stays stopped: the
    rows after it say so at once, and what it already measured stands."""
    if session.stopped:
        return {'wall': None, 'block': None, 'instructions': None, 'failed': f'not run: {session.stopped}'}
    print(f'    {session.engine.name} {name}', flush=True)
    try:
        return function()
    except SESSION_FAILURES as error:
        try:
            for _ in range(3):
                session.engine.keys(parity_live.C('g'))
            session.ask(recover, 15)
        except SESSION_FAILURES:
            session.stopped = f'the session stopped answering in {name.removeprefix("session ")}'
        return {'wall': None, 'block': None, 'instructions': None, 'failed': f'not run: {str(error)[:160]}'}


def session_rows(ctx, wanted, inputs):
    """The shipped-defaults rows WANTED: one session per engine."""
    results = {engine: ctx.attempt(engine, lambda e=engine: shipped_figures(ctx, e, inputs, wanted)) for engine in ctx.compared}
    romacs_note = results['romacs'].pop('romacs_note', 'Romacs: loads not checked, the session stopped answering')
    rows = []
    for name, *_ in SESSION_SHIPPED:
        if name not in wanted:
            continue
        engines = {e: results[e] if results[e].get('failed') else results[e][name] for e in ctx.compared}
        note = '; '.join(f'{ENGINE_NAMES[e]}: {engines[e]["failed"]}' for e in ('gnu', 'romacs') if engines[e].get('failed'))
        own = '; each engine on its own shipped themes' if 'theme switch' in name else ''
        made = row(name, 'session', 'run', engines['gnu'], engines['romacs'], ctx.runs,
                   '; '.join(filter(None, [note, check_note(engines), romacs_note + own])))
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


# The two completions the comparison session's prompts run under: GNU's
# fido-vertical in both engines, and Romacs's defaults, its layer with the
# standard styles (fzf first) the comparison session left.
FIDO = '(fido-vertical-mode 1)'
LAYER = ("(progn (custom-reevaluate-setting 'completion-styles)"
         " (custom-reevaluate-setting 'completion-category-overrides) (romacs-complete-mode 1))")
QUIET = '(setq auto-save-default nil create-lockfiles nil make-backup-files nil) (blink-cursor-mode -1)'
RECOVER = '(progn (delete-other-windows) (switch-to-buffer "*scratch*") t)'
TEXT = ''.join(f'line {i} of some ordinary prose text, about sixty bytes long\n' for i in range(200))


def key_rows(ctx, wanted, inputs):
    """The comparison session's WANTED per-key rows: one session per engine.
    A row an engine has no figures for (Romacs's own completion, in GNU) has
    no column there."""
    results = {engine: ctx.attempt(engine, lambda e=engine: comparison_figures(ctx, e, inputs[0], wanted))
               for engine in ctx.compared}
    romacs_note = results['romacs'].pop('romacs_note', 'Romacs: loads not checked, the session stopped answering')
    names = set(TEXT_KEYS) | {c[0] for c in CODE_KEYS + COMPLETIONS}
    rows = []
    for name in (n for n, _ in catalogue() if n in wanted and n in names):
        engines = {e: r if r.get('failed') else r[name] for e, r in results.items() if r.get('failed') or name in r}
        failed = '; '.join(f'{ENGINE_NAMES[e]}: {f["failed"]}' for e, f in engines.items() if f.get('failed'))
        made = row(name, 'gui', 'key', engines.get('gnu'), engines.get('romacs'), ctx.runs,
                   '; '.join(filter(None, [failed, check_note(engines), key_note(name, engines), romacs_note])))
        rows.append(with_neomacs(made, engines))
    return rows


def key_note(name, engines):
    """What a per-key row's note says of how it ran."""
    samples = ', '.join(f'{ENGINE_NAMES[e]} {f["samples"]}' for e, f in engines.items() if f.get('samples') is not None)
    note = f'{KEYS_PER_ROUND} keys a round, latencies {samples}'
    completion_ = next((c for c in COMPLETIONS if c[0] == name), None)
    if completion_:
        candidates = ', '.join(f'{ENGINE_NAMES[e]} {f["candidates"]}' for e, f in engines.items() if 'candidates' in f)
        own = "Romacs only: its default completion (the layer, fzf); " if completion_[3] == 'layer' else 'fido-vertical; '
        note = f'{own}the keys of {completion_[2]}, latencies {samples}; candidates {candidates}'
    code = next((c for c in CODE_KEYS if c[0] == name), None)
    if code:
        note = f'{"from the middle" if code[3] == "middle" else "from the top"}, each round in the file visited afresh; {note}'
    return note


def comparison_figures(ctx, engine, repo, wanted):
    """ENGINE's WANTED per-key rows, in one session with GNU's completion."""
    figures = {}
    session = Session(ctx, engine)
    try:
        text = session.root / 'work/text.txt'
        text.write_text(TEXT)
        session.ask(f'(progn {QUIET} {FIDO} (setq icomplete-compute-delay 0) t)')
        time.sleep(2)
        rate = session.idle_rate()
        rounds = ctx.runs
        for name, group in TEXT_KEYS.items():
            if name in wanted:
                figures[name] = session_row(session, name, lambda g=group: text_keys(session, text, g, rate, rounds), RECOVER)
        for name, path, ts, where, group, gap, _ in CODE_KEYS:
            if name in wanted:
                figures[name] = session_row(session, name, lambda p=repo / path, s=ts, w=where, g=group, k=gap:
                                            code_keys(session, p, s, w, g, k, rate, rounds), RECOVER)
        for name, prompt, typed, display, _ in COMPLETIONS:
            if name in wanted and (display != 'layer' or engine == 'romacs'):
                figures[name] = session_row(session, name, lambda p=prompt, y=typed, d=display:
                                            completion(session, ctx.fixtures / 'd5000', p, y, LAYER if d == 'layer' else FIDO,
                                                       rate, rounds), RECOVER)
        if engine == 'romacs' and not session.stopped:
            figures['romacs_note'] = session.romacs_note()
    finally:
        session.stop()
    return figures


def phase_medians(phases, **extra):
    """One row's figures from its rounds' phases."""
    return {'instructions': median(p['instructions'] for p in phases), 'wall': median(p['wall'] for p in phases),
            'samples': sum(p['samples'] for p in phases), **extra}


def text_keys(session, path, group, rate, rounds):
    """GROUP, KEYS_PER_ROUND times a round, from the start of the text PATH."""
    phases = []
    for _ in range(rounds):
        session.ask(f'(progn (find-file {parity_live.lisp_string(str(path))}) (goto-char (point-min)) t)')
        time.sleep(.5)
        phases.append(session.phase([group] * KEYS_PER_ROUND, .12, rate))
    return phase_medians(phases, check=None)


def code_keys(session, path, ts, where, group, gap, rate, rounds):
    """GROUP, KEYS_PER_ROUND times a round, GAP seconds apart, in PATH visited
    afresh each round (C in c-ts-mode with TS), from WHERE: `middle' or `top'."""
    phases, modes = [], []
    for _ in range(rounds):
        modes.append(session.ask(f'(perfbench-gui-visit {parity_live.lisp_string(str(path))} {"t" if ts else "nil"}'
                                 f' (quote {where}))', 120))
        time.sleep(1.5)  # its first screen fontified, a background parse done
        phases.append(session.phase([group] * KEYS_PER_ROUND, gap, rate))
    return phase_medians(phases, check=modes[0])


def completion(session, directory, prompt, typed, display, rate, rounds):
    """The keys of TYPED, one a key, in the prompt PROMPT opens under DISPLAY,
    from DIRECTORY: what each key's completion costs."""
    session.ask(f'(progn (setq icomplete-compute-delay 0) {display}'
                f' (with-current-buffer (window-buffer (selected-window)) (setq default-directory'
                f' {parity_live.lisp_string(str(directory) + "/")})) t)')
    phases, check, candidates = [], None, None
    for _ in range(rounds):
        session.engine.keys(prompt)
        parity_live.wait(lambda: session.ask('(minibufferp (window-buffer (selected-window)))'), 30,
                         'the prompt', session.engine.process)
        time.sleep(1)
        phases.append(session.phase([['-k', KEYSYM[c]] if c in KEYSYM else [c] for c in typed], .3, rate))
        check = os.path.basename(session.ask('(minibuffer-contents-no-properties)'))
        candidates = session.ask('(safe-length (completion-all-sorted-completions))')
        session.engine.keys(CTRL(['g']))
        time.sleep(.5)
    return phase_medians(phases, check=check, candidates=candidates)


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
    """ROWS with Romacs/GNU ratios, and each figure's change against the first
    of PREVIOUS (earlier results, newest first) that has it: a run of a few
    rows, or a wall refused for load, leaves the others their last figures."""
    before = {}
    for results in previous or ():
        for r in results['rows']:
            for metric, value in (r.get('romacs') or {}).items():
                if value is not None:
                    before.setdefault((r['row'], metric), value)
    for r in rows:
        for metric in ('instructions', 'wall', 'block'):
            value = (r['romacs'] or {}).get(metric)
            r[f'{metric}_ratio'] = ratio(value, (r['gnu'] or {}).get(metric))
            r[f'{metric}_change'] = change(value, before.get((r['row'], metric)))
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
             f'GNU `{meta["gnu"]}`.  Runs {meta["runs"]}; rows {", ".join(meta.get("only") or ["all"])}; '
             f'{meta.get("seconds", 0) // 60:.0f} min {meta.get("seconds", 0) % 60:.0f} s; '
             f'load {meta["load_start"]:.1f} at start, {meta["load_end"]:.1f} at end, walls refused above '
             f'{meta["max_load"]}.  Change: against the newest earlier run with the row (newest {meta["previous"] or "none"}).  Romacs Lisp: {meta["romacs_lisp"]}.', '',
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
    if results['meta'].get('timings'):
        lines.append('\nTime per runner: ' + ', '.join(f'{label} {seconds:.0f} s' for label, seconds in results['meta']['timings']))
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


def previous_results(out_root, explicit, current, romacs_lisp):
    """The earlier results to compare with, newest first: EXPLICIT alone, or
    every saved run but CURRENT whose Romacs read its Lisp the same way."""
    if explicit:
        return [json.loads(Path(explicit).read_text())]
    found = [json.loads(p.read_text()) for p in out_root.glob('*/*/results.json') if p.parent != current]
    return sorted((r for r in found if r['meta'].get('romacs_lisp') == romacs_lisp),
                  key=lambda r: r['meta']['started'], reverse=True)


# --- main -----------------------------------------------------------------------

class Context:
    def __init__(self, args, run_dir, cache, fixtures_dir, runtime, romacs, store):
        """STORE: the compiled store ROMACS loads from; None for --romacs-source."""
        self.args, self.run_dir, self.cache, self.fixtures, self.store = args, run_dir, cache, fixtures_dir, store
        self.runs, self.tools = args.runs, args.tools
        self.engines = {'gnu': args.gnu, 'romacs': str(romacs)}
        self.gnu_build = gnu_build(args.gnu)
        if args.neomacs:
            self.engines['neomacs'] = str(Path(args.neomacs).expanduser())
        self.compared = tuple(self.engines)
        self.memory = {'neomacs': NEOMACS_MEMORY}
        self.engine_env = {'gnu': {}, 'neomacs': {}, 'romacs': {'ROMACS_RUNTIME_ROOT': str(runtime),
                                                 'ROMACS_LISP_DIR': str(runtime / 'lisp/gnu'),
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

    def scratch(self, suffix):
        """A fresh path in the work directory."""
        return self.work / f'{time.monotonic_ns()}{suffix}'

    def log(self, name, text):
        logs = self.run_dir / 'logs'
        logs.mkdir(exist_ok=True)
        (logs / (re.sub(r'[^A-Za-z0-9_.-]', '-', name) + '.log')).write_text(text)


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
        if candidate and (Path(candidate) / 'lisp/gnu').is_dir() and (Path(candidate) / 'lisp/romacs').is_dir():
            return Path(candidate).resolve()
    raise SystemExit('no runtime: give --runtime DIR (holding lisp/gnu/ and lisp/romacs/)')


def commit_of(args):
    if args.commit:
        return args.commit
    build = Path(args.romacs).resolve().parent / 'build.json'
    if build.exists():
        return json.loads(build.read_text())['commit']
    raise SystemExit('no build.json beside the binary: give --commit SHA')


def plan(ctx, wanted, every_member):
    """(rows of one runner, the runner) for every runner with a row WANTED: a
    runner makes the rows wanted of its own, sharing its sessions and setups
    among them.  EVERY_MEMBER: the elisp-benchmarks runner makes a row of
    each member the suite defines."""
    batch = None

    def batch_():
        nonlocal batch
        batch = batch or Batch(ctx)
        return batch
    inputs = lambda: session_inputs(ctx, wanted)  # noqa: E731
    steps = [([name], lambda n=name, w=workload: [batch_().workload(n, w)]) for name, workload, _ in BATCH]
    steps += [([name], lambda n=name, p=path, m=mode, o=op, w=what: [syntax_row(batch_(), n, p, m, o, w)])
              for name, _, path, mode, op, _, what in syntax_rows()]
    steps += [([n for n, _ in MICRO], lambda: micro_rows(ctx, batch_(), wanted)),
              ([n for n, _ in UNCOMPILED], lambda: uncompiled_rows(ctx, batch_(), wanted)),
              (sorted({f'elb {m}' for m in ELB_DEFAULT} | {w for w in wanted if w.startswith('elb ')}),
               lambda: elb_rows(ctx, batch_(), None if every_member else
                                {w.removeprefix('elb ') for w in wanted if w.startswith('elb ')})),
              ([n for n, _ in LAUNCH], lambda: launch_rows(ctx, wanted)),
              ([n for n, _ in catalogue() if n in TEXT_KEYS or n in {c[0] for c in CODE_KEYS + COMPLETIONS}],
               lambda: key_rows(ctx, wanted, inputs())),
              ([n for n, *_ in SESSION_SHIPPED], lambda: session_rows(ctx, wanted, inputs()))]
    return [(names, function) for names, function in steps if any(n in wanted for n in names)]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs')
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'),
                        help='the GNU engine (default: $ROMACS_GNU_ORACLE, the pinned oracle)')
    parser.add_argument('--runtime')
    parser.add_argument('--commit')
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--only', nargs='+', help='areas, row names or globs, comma or space separated')
    parser.add_argument('--list', action='store_true', help='print every row and its areas, and exit')
    parser.add_argument('--max-load', type=float, default=8.0)
    parser.add_argument('--out', default=os.environ.get('ROMACS_PERFBENCH_OUT', '~/.cache/romacs/perf-bench'))
    parser.add_argument('--previous', help='results.json to compare with (default: the newest saved run)')
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--keep', action='store_true', help='keep the private trees')
    parser.add_argument('--neomacs', help='add neomacs as a third engine, as this program installs')
    parser.add_argument('--neomacs-commit', help='the commit --neomacs was built from, recorded with its build')
    parser.add_argument('--romacs-source', action='store_true',
                        help='measure Romacs from source, with no compiled store or world, not as an installation runs')
    args = parser.parse_args()
    if args.list:
        print('\n'.join(f'{name:48} {" ".join(areas)}' for name, areas in catalogue()))
        print(f'{"elb MEMBER":48} elb (every member: --only elb)')
        return 0
    wanted, every_member = selection(args.only, catalogue())
    if not args.romacs or not args.gnu:
        raise SystemExit('needs --romacs, and --gnu or $ROMACS_GNU_ORACLE')
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
    os.environ.update(ROMACS_RUNTIME_ROOT=str(runtime), ROMACS_LISP_DIR=str(runtime / 'lisp/gnu'),
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
            'gnu': args.gnu, 'gnu_build': ctx.gnu_build, 'runs': args.runs, 'only': args.only, 'max_load': args.max_load,
            'load_start': load(), 'fixtures': pins, 'cpus': os.cpu_count()}
    print(f'perfbench {commit[:12]} -> {run_dir}; load {meta["load_start"]:.1f}', flush=True)
    rows, timings, began = [], [], time.time()
    try:
        for names, function in plan(ctx, set(wanted), every_member):
            mine = [n for n in names if n in wanted]
            label = mine[0] if len(mine) == 1 else f'{mine[0]} and {len(mine) - 1} more'
            started_at, watch = time.time(), LoadWatch()
            try:
                made = watch.during(function)
            except Exception as error:  # noqa: BLE001 -- one runner's failure is reported, the rest still run
                ctx.skipped.append(f'{label}: {error}')
                ctx.log(f'{label} error', traceback.format_exc())
                print(f'  {label}: not run: {error}', flush=True)
                continue
            for r in made:
                r.setdefault('load', watch.peak)
            rows += made
            timings.append([label, round(time.time() - started_at, 1)])
            print(f'  {label}: {time.time() - started_at:.0f}s, load up to {watch.peak:.1f}', flush=True)
    finally:
        if not args.keep:
            shutil.rmtree(ctx.work, ignore_errors=True)
    meta['seconds'], meta['timings'] = round(time.time() - began), timings
    meta['load_end'] = load()
    meta['elb_native'] = ctx.elb_native
    meta['header_bytes'] = ctx.header_bytes
    meta['history_commits'] = ctx.history_commits
    meta['history_commit'] = UPSTREAM
    previous = previous_results(out_root, args.previous, run_dir, meta['romacs_lisp'])
    meta['previous'] = previous[0]['meta']['started'] if previous else None
    results = {'meta': meta, 'skipped': ctx.skipped, 'rows': with_ratios(gate_walls(rows, args.max_load), previous)}
    (run_dir / 'results.json').write_text(json.dumps(results, indent=1) + '\n')
    text = table(results)
    (run_dir / 'table.md').write_text(text)
    print(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
