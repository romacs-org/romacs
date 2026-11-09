#!/usr/bin/env python3
"""Run GNU Emacs's own test suite (gnu-tests/test, pristine at the pin) on romacs.

    python3 gnu-tests/run.py run src/fns-tests.el           # one file, by hand
    python3 gnu-tests/run.py run --all --jobs 16            # the whole suite
    python3 gnu-tests/run.py baseline src/fns-tests.el      # (re)record GNU's verdicts
    python3 gnu-tests/run.py check-ledger                   # validate EXCLUSIONS.toml
    python3 gnu-tests/run.py verify-pin                     # the import is pristine

Every deftest gets exactly one VERDICT:
    passed            romacs gave GNU's own as-expected outcome
    failed            an assertion (should/should-not/should-error) failed
    errored           any other signal, a VM fault, a panic or a timeout
    skipped           the test skipped itself
    not-yet-runnable  romacs never ran it: its file, its definition or the
                      harness could not get that far
and every verdict other than `passed` gets exactly one CATEGORY:
    intentional-divergence  ONLY via an EXCLUSIONS.toml row bound to a
                            DIVERGENCES.md entry (validated; see below)
    out-of-scope            ONLY via an EXCLUSIONS.toml row, same validation
    not-yet-implemented     names what is missing (`missing`), derived
                            mechanically from the failure, never asserted
    failing                 everything else
plus two neutral categories that are not romacs's doing and are reported apart:
    gnu-fails-too           GNU itself does not give the expected result here
    gnu-skips-too           GNU skips it too
The ONLY way to put a test in an exclusion category is a ledger row that names
an existing DIVERGENCES.md id, a Lisp symbol that row lists AND that the test's
source uses, and GNU's behaviour, ours and why it is intentional. A row that
fails any check stops the run (exit 2) before anything is measured; a row for a
test that now passes stops it too. There is no other skip switch.

Exit status: 0 every selected test passed; 1 at least one did not; 2 usage or
an invalid ledger; 3 the import drifted from the pin or the GNU baseline is
stale; 4 infrastructure failure (build, extraction).
"""
from __future__ import annotations

import argparse
import collections
import concurrent.futures
import hashlib
import json
import os
import pathlib
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import tomllib

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
TESTS = HERE / 'test'
PIN_FILE = HERE / 'PIN.json'
LEDGER = HERE / 'EXCLUSIONS.toml'
BASELINE = HERE / 'baseline'
HARNESS = HERE / 'harness' / 'romacs-mini-ert.el'
# Loaded by the adapter right after HARNESS (mini mode): records every
# should-error that absent machinery satisfied. A separate file so HARNESS,
# whose digest every GNU baseline is bound to, stays byte-identical.
HARNESS_AUDIT = HERE / 'harness' / 'romacs-mini-ert-audit.el'
REFERENCE = ROOT / 'parity-reference.toml'
EXTRACT = HERE / 'tools' / 'extract-forms.el'
GNU_RUN = HERE / 'tools' / 'gnu-run.el'
# GNU lisp/loadup.el order, loaded as far as it reaches (same list and rule as
# the WARM census, tools/lisp-census BOOTSTRAP_CHAIN; a test pins them).
BOOTSTRAP_CHAIN = ['emacs-lisp/debug-early', 'emacs-lisp/byte-run', 'emacs-lisp/backquote', 'subr',
                   'keymap', 'version', 'widget', 'custom', 'emacs-lisp/map-ynp', 'international/mule',
                   'international/mule-conf', 'env', 'format', 'bindings', 'window', 'files',
                   'emacs-lisp/macroexp']
# MEASUREMENT REGIME: retains bindings.el's failed prefix before easy-mmode.
# Not an accepted bootstrap; never replace the STANDARD WARM census chain.
ORDERED_BOOTSTRAP_CHAIN = [
    'emacs-lisp/debug-early', 'emacs-lisp/byte-run', 'emacs-lisp/backquote',
    'subr', 'keymap', 'emacs-lisp/pcase', 'emacs-lisp/gv',
    'emacs-lisp/inline', 'custom', 'bindings', 'emacs-lisp/easy-mmode', 'emacs-lisp/cl-lib',
    'faces', 'cus-face', 'button',
    'emacs-lisp/debug', 'emacs-lisp/backtrace', 'emacs-lisp/ewoc',
    'emacs-lisp/find-func', 'emacs-lisp/pp', 'emacs-lisp/map', 'help-mode',
]
# The gnu-ert regime: GNU's whole preloaded world, as the dumped Emacs the
# GNU tests assume has it (rlisp/romacs-loadup-bootstrap.el).
ERT_SOURCE_BOOTSTRAP = ['rlisp/romacs-loadup-bootstrap.el']
ERT_REPORT_BOOTSTRAP = ['gnu-lisp/help-fns.el', 'rlisp/romacs-cl-print.el']


def source_bootstrap_chain(args):
    if getattr(args, 'bootstrap', 'standard') == 'ert':
        return ERT_SOURCE_BOOTSTRAP + (ERT_REPORT_BOOTSTRAP
                                     if getattr(args, 'harness', 'real') == 'real' else [])
    return ORDERED_BOOTSTRAP_CHAIN if getattr(args, 'bootstrap', 'standard') == 'ordered' else BOOTSTRAP_CHAIN


# GNU test/Makefile.in prunes these directories when it collects test files.
PRUNED_DIRS = ('manual', 'data', 'infra')
VERDICTS = ('passed', 'failed', 'errored', 'skipped', 'not-yet-runnable')
CATEGORIES = ('intentional-divergence', 'out-of-scope', 'excluded-unverified-row', 'not-yet-implemented',
              'failing', 'gnu-fails-too', 'gnu-skips-too', 'gnu-not-recorded', 'vacuous-pass', 'unaudited-pass')
# Conditions that mean "the machinery under test does not exist" when a
# should-error is satisfied by them and GNU caught something else.
ABSENT_MACHINERY = ('void-function', 'void-variable', 'invalid-function')
SUSPECT_FILE = HERE / 'SUSPECT-DIVERGENCES.toml'
GNU_MIRROR = pathlib.Path(os.environ.get('ROMACS_GNU_SOURCE', pathlib.Path(os.environ.get('GNU_ORACLE_ROOT', pathlib.Path.home() / '.cache/gnu-emacs-pin')) / 'src'))
GNU_SOURCE = re.compile(r'^(src/[\w-]+\.c|lisp/[\w/+-]+\.el):(\d+)-(\d+)$')
# A top-level deftest GNU's baseline run never reported (the file timed out or
# its load stopped first).  There is no specification for it, so it can never
# be `passed`; it stays in the denominator.
NOT_RECORDED = {'outcome': 'not-recorded', 'as_expected': False, 'expected': None, 'tags': [], 'detail': None}
EXCLUSION_CATEGORIES = ('intentional-divergence', 'out-of-scope')
PIN_COMMIT = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
DIVERGENCE_ID = re.compile(r'^[A-Z][A-Z0-9]*(-[A-Z0-9]+)*$')
UNSUPPORTED = re.compile(r'(not (yet )?supported|unsupported|not implemented)', re.I)


class Refusal(Exception):
    """A condition under which no verdict may be produced at all."""

    def __init__(self, status: int, message: str):
        super().__init__(message)
        self.status = status


def git(*args: str, root: pathlib.Path = ROOT) -> str:
    return subprocess.run(['git', '-C', str(root), *args], check=True,
                          capture_output=True, text=True).stdout.strip()


def git_or_none(*args: str):
    try:
        return git(*args)
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None


def git_blob_id(data: bytes) -> str:
    return hashlib.sha1(b'blob %d\0' % len(data) + data).hexdigest()


def git_tree_id(directory: pathlib.Path) -> str:
    """The id git would give DIRECTORY's working content, computed without git.

    Content-addressed exactly as git does (blob/tree object headers, modes
    100644/100755/120000/40000, entries ordered by name with directories
    compared as NAME/). So it needs no repository, sees uncommitted edits and
    stray files, and equals GNU's own `45b0f7699df:test` only for identical
    bytes, names and modes.
    """
    entries = []
    for child in directory.iterdir():
        if child.is_symlink():
            mode, oid, key = b'120000', git_blob_id(os.readlink(child).encode()), child.name
        elif child.is_dir():
            if not any(c.is_file() or c.is_symlink() for c in child.rglob('*')):
                continue  # git records no empty directory
            mode, oid, key = b'40000', git_tree_id(child), child.name + '/'
        else:
            executable = child.stat().st_mode & 0o100
            mode, oid, key = (b'100755' if executable else b'100644'), git_blob_id(child.read_bytes()), child.name
        entries.append((key.encode(), mode + b' ' + child.name.encode() + b'\0' + bytes.fromhex(oid)))
    body = b''.join(e for _, e in sorted(entries))
    return hashlib.sha1(b'tree %d\0' % len(body) + body).hexdigest()


# --------------------------------------------------------------------------
# The pristine import

def pin(root: pathlib.Path = ROOT) -> dict:
    return json.loads((root / 'gnu-tests' / 'PIN.json').read_text())


def verify_pin(root: pathlib.Path = ROOT) -> dict:
    """Refuse unless gnu-tests/test is byte-for-byte GNU's test/ at the pin.

    The git tree id of the WORKING content of gnu-tests/test (computed by
    git_tree_id, no repository needed, so it also holds in the gate's source
    mirror) must equal GNU's own tree id for `test/` at the pinned commit.
    Any edit, addition, deletion or mode change anywhere under it changes the
    id.
    """
    p = pin(root)
    actual = git_tree_id(root / 'gnu-tests' / 'test')
    problems = []
    if actual != p['test_tree']:
        problems.append(f'its content hashes to git tree {actual}; GNU {p["gnu_commit"][:11]}:test is '
                        f'{p["test_tree"]}. Something under gnu-tests/test was edited, added, '
                        'removed or re-moded (see `git status gnu-tests/test`).')
    if problems:
        raise Refusal(3, 'gnu-tests/test is not the pristine GNU import:\n' + '\n'.join(problems)
                      + '\nNever edit an imported test in place; record divergences in EXCLUSIONS.toml.')
    return p


# Tests run in a scratch COPY of the import, never in gnu-tests/test itself:
# GNU's own tests write beside themselves (bytecomp-resources/*.elc, gnus
# random_seed), and a run must not be able to make the import non-pristine.
RUN_TESTS = TESTS


class ScratchTree:
    """Copy gnu-tests/test to a scratch directory, prove the copy pristine."""

    def __enter__(self):
        global RUN_TESTS
        import shutil
        self.tmp = tempfile.TemporaryDirectory(prefix='gts-tree-')
        copy = pathlib.Path(self.tmp.name) / 'test'
        shutil.copytree(TESTS, copy, symlinks=True)
        tree = git_tree_id(copy)
        if tree != pin()['test_tree']:
            raise Refusal(3, f'scratch copy of the import hashes to {tree}, not the pin')
        RUN_TESTS = copy
        return copy

    def __exit__(self, *exc):
        global RUN_TESTS
        RUN_TESTS = TESTS
        self.tmp.cleanup()


def blob_of(rel: str) -> str:
    return git_blob_id((TESTS / rel).read_bytes())


def suite_files() -> list[str]:
    """Every file of the import that defines a test: the 565 that hold GNU's
    6987 textual `ert-deftest' forms.  Five of them (test/manual/*, and
    erc-d-tests.el under a resources directory) are outside `make check';
    in_make_check() says which, and the summary counts both scopes."""
    return sorted(str(path.relative_to(TESTS)) for path in TESTS.rglob('*.el')
                  if '(ert-deftest' in path.read_text(errors='replace'))


def in_make_check(rel: str) -> bool:
    """GNU test/Makefile.in ELFILES prunes manual, data, infra and *resources."""
    return not any(part in PRUNED_DIRS or part.endswith('resources') for part in pathlib.Path(rel).parts[:-1])


def normalize(name: str) -> str:
    p = pathlib.Path(name)
    if p.is_absolute():
        p = p.resolve().relative_to(TESTS)
    elif p.parts[:2] == ('gnu-tests', 'test'):
        p = pathlib.Path(*p.parts[2:])
    elif p.parts[:1] == ('test',):
        p = pathlib.Path(*p.parts[1:])
    if not (TESTS / p).is_file():
        raise Refusal(2, f'no such GNU test file: {name}')
    return str(p)


# --------------------------------------------------------------------------
# The exclusion ledger

def divergence_rows() -> dict[str, dict]:
    sys.path.insert(0, str(ROOT / 'tools/lib'))
    try:
        import divergence_index  # the one parser of the prose ledgers
    finally:
        sys.path.pop(0)
    rows = divergence_index.parse(ROOT / 'DIVERGENCES.md', 'DIVERGENCES.md')
    for p in sorted(ROOT.glob('crates/*/GNU-DIVERGENCES.md')):
        rows += divergence_index.parse(p, str(p.relative_to(ROOT)))
    out = {}
    for r in rows:
        ident = re.sub(r'^\[([^\]]+)\]\(.*\)$', r'\1', r['id'] or '')
        if DIVERGENCE_ID.match(ident):
            out[ident] = r
    return out


LEDGER_FIELDS = ('file', 'test', 'category', 'divergence', 'symbol', 'gnu', 'romacs', 'why',
                 'gnu_source', 'gnu_source_sha256', 'verified_by')


def suspect_ids() -> dict[str, str]:
    return tomllib.loads(SUSPECT_FILE.read_text()).get('suspect', {})


def gnu_lines_sha256(citation: str, commit: str) -> str | None:
    """sha256 of the cited lines of GNU's file at the pin, or None when the
    GNU source mirror is unavailable (the row then cannot count as verified)."""
    m = GNU_SOURCE.match(citation)
    try:
        text = subprocess.run(['git', '-C', str(GNU_MIRROR), 'show', f'{commit}:{m.group(1)}'], check=True,
                              capture_output=True).stdout.decode('utf-8', 'replace')
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None
    lines = text.split('\n')
    a, b = int(m.group(2)), int(m.group(3))
    if not 1 <= a <= b <= len(lines):
        return 'out-of-range'
    return hashlib.sha256(('\n'.join(lines[a - 1:b]) + '\n').encode()).hexdigest()


def load_ledger(path: pathlib.Path = LEDGER, plans: dict | None = None) -> dict[tuple[str, str], dict]:
    """Parse and VALIDATE every row; refuse the whole ledger on any defect.

    plans maps file -> extraction plan; when given, each row's test must be a
    top-level deftest of that file whose source contains the row's symbol.
    """
    if not path.exists():
        return {}
    data = tomllib.loads(path.read_text())
    unknown = set(data) - {'exclude'}
    errors = [f'unknown top-level key {k!r}' for k in sorted(unknown)]
    divergences = divergence_rows()
    suspects = suspect_ids()
    rows: dict[tuple[str, str], dict] = {}
    for i, row in enumerate(data.get('exclude', []), 1):
        where = f'row {i} ({row.get("file")}::{row.get("test")})'
        extra = set(row) - set(LEDGER_FIELDS)
        if extra:
            errors.append(f'{where}: unknown field(s) {sorted(extra)}')
        for field in LEDGER_FIELDS:
            value = row.get(field)
            if not isinstance(value, str) or not value.strip():
                errors.append(f'{where}: missing or empty `{field}`')
        if any(not isinstance(row.get(f), str) for f in LEDGER_FIELDS):
            continue
        for field in ('gnu', 'romacs', 'why'):
            if len(row[field].split()) < 6:
                errors.append(f'{where}: `{field}` must explain, in a sentence, not {row[field]!r}')
        verification = []
        if not GNU_SOURCE.match(row['gnu_source']):
            errors.append(f'{where}: gnu_source must cite GNU implementation at the pin as '
                          f'src/FILE.c:A-B (or lisp/FILE.el:A-B), not {row["gnu_source"]!r}')
        else:
            actual = gnu_lines_sha256(row['gnu_source'], PIN_COMMIT)
            if actual is None:
                verification.append('the GNU source mirror is unavailable, so the citation was not re-checked')
            elif actual != row['gnu_source_sha256']:
                errors.append(f'{where}: {row["gnu_source"]} at {PIN_COMMIT[:11]} hashes to {actual}, not '
                              f'{row["gnu_source_sha256"]}; the cited GNU code is not what was verified')
        if row['divergence'] in suspects:
            verification.append(f'ledger entry {row["divergence"]} is listed as suspect: '
                                f'{suspects[row["divergence"]]}')
        if row['category'] not in EXCLUSION_CATEGORIES:
            errors.append(f'{where}: category must be one of {EXCLUSION_CATEGORIES}, not {row["category"]!r}')
        if re.search(r'[*?\[\]]', row['test'] + row['file']):
            errors.append(f'{where}: no globs; one row names exactly one deftest')
        div = divergences.get(row['divergence'])
        if div is None:
            errors.append(f'{where}: divergence {row["divergence"]!r} is not an entry in DIVERGENCES.md '
                          f'(or a crate GNU-DIVERGENCES.md); record the divergence there first')
        elif row['symbol'] not in div['symbols']:
            errors.append(f'{where}: symbol {row["symbol"]!r} is not one of the symbols ledger entry '
                          f'{row["divergence"]} covers {div["symbols"]}')
        try:
            rel = normalize(row['file'])
        except Refusal as e:
            errors.append(f'{where}: {e}')
            continue
        if plans is not None and rel in plans:
            sources = [f.get('source') or '' for f in plans[rel]['forms'] if f.get('deftest') == row['test']]
            if not sources:
                errors.append(f'{where}: {rel} has no top-level deftest {row["test"]!r}')
            elif not re.search(r'(?<![-\w])' + re.escape(row['symbol']) + r'(?![-\w])', sources[0]):
                errors.append(f'{where}: the test never mentions {row["symbol"]!r}; the cited divergence '
                              'cannot be what it exercises')
        key = (rel, row['test'])
        if key in rows:
            errors.append(f'{where}: duplicate row')
        rows[key] = dict(row, file=rel, verified=not verification, unverified_because=verification)
    if errors:
        raise Refusal(2, f'{path} is invalid; nothing was measured:\n  ' + '\n  '.join(errors))
    return rows


# --------------------------------------------------------------------------
# GNU: extraction and the oracle baseline

def emacs() -> str:
    # The gate exports the pinned oracle as ROMACS_FORCE_ORACLE_PATH.
    return os.environ.get('GNU_EMACS') or os.environ.get('ROMACS_FORCE_ORACLE_PATH') or 'emacs'


def gnu_env(scratch: pathlib.Path) -> dict:
    env = dict(os.environ, HOME=str(scratch), EMACS_TEST_DIRECTORY=str(RUN_TESTS), LANG='C')
    env.pop('EMACSLOADPATH', None)
    env.pop('EMACS_TEST_JUNIT_REPORT', None)
    return env


def extract(rel: str, scratch: pathlib.Path, binary: str | None = None) -> dict:
    out = scratch / (rel.replace('/', '__') + '.plan.json')
    p = subprocess.run([binary or emacs(), '-Q', '--batch', '-l', str(EXTRACT), str(RUN_TESTS / rel), str(out)],
                       capture_output=True, text=True, timeout=120, env=gnu_env(scratch))
    if p.returncode != 0 or not out.exists():
        raise Refusal(4, f'GNU could not extract the forms of {rel} (exit {p.returncode}): {p.stderr[-600:]}')
    plan = json.loads(out.read_text())
    plan['path'] = str(out)
    return plan


def gnu_run(mode: str, rel: str, scratch: pathlib.Path, timeout: float, plan: dict | None = None,
            binary: str | None = None) -> dict:
    cmd = [binary or emacs(), '-Q', '--batch', '-L', str((RUN_TESTS / rel).parent), '-l', str(GNU_RUN), mode, str(RUN_TESTS / rel)]
    if mode == 'mini':
        cmd += [plan['path'], str(HARNESS)]
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           cwd=(RUN_TESTS / rel).parent, env=gnu_env(scratch), errors='replace')
        stdout, rc, timed_out = p.stdout, p.returncode, False
    except subprocess.TimeoutExpired as e:
        stdout = (e.stdout or b'').decode(errors='replace') if isinstance(e.stdout, bytes) else (e.stdout or '')
        rc, timed_out = None, True
    events, garbled = [], 0
    for line in stdout.splitlines():
        if line.startswith('{"event"'):
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:  # a line cut short by the timeout kill
                garbled += 1
    results = {e['test']: {k: e.get(k) for k in ('outcome', 'expected', 'as_expected', 'tags', 'detail')}
               for e in events if e['event'] == 'result'}
    for event in events:
        if event.get('event') == 'result' and 'caught' in event:
            results[event['test']]['caught'] = event['caught']
        if event.get('event') == 'result' and event.get('vacuous_void_function'):
            results[event['test']]['vacuous_void_function'] = event['vacuous_void_function']
    load = next((e for e in events if e['event'] == 'load'), None)
    return {'exit': rc, 'timed_out': timed_out, 'garbled_lines': garbled,
            'complete': any(e['event'] == 'done' for e in events),
            'load_ok': None if load is None else load['ok'],
            'load_error': None if load is None else load['error'], 'results': results}


def gnu_identity() -> dict:
    exe = subprocess.run(['sh', '-c', f'command -v {emacs()}'], capture_output=True, text=True).stdout.strip()
    version = subprocess.run([emacs(), '--version'], capture_output=True, text=True).stdout.splitlines()[0]
    real = os.path.realpath(exe)
    return {'emacs': real, 'version': version,
            'sha256': hashlib.sha256(pathlib.Path(real).read_bytes()).hexdigest()}


def baseline_path(rel: str) -> pathlib.Path:
    return BASELINE / (rel + '.json')


def pinned_oracle() -> dict:
    """The pinned GNU that may record baselines: $ROMACS_GNU_ORACLE, asked who
    it is (tools/lib/gnu_oracle.py), and its digest equal to the pin's."""
    sys.path.insert(0, str(ROOT / 'tools/lib'))
    import gnu_oracle
    try:
        ident = gnu_oracle.require()
    except SystemExit as e:  # gnu_oracle refuses by raising OracleError(SystemExit)
        raise Refusal(3, f'baselines are recorded only by the pinned GNU: {e}') from None
    if ident['sha256'] != pinned_gnu_sha256():
        raise Refusal(3, f'{ident["path"]} reports the pinned revision but its sha256 {ident["sha256"]} is not '
                         f'parity-reference.toml executable_sha256 {pinned_gnu_sha256()}; refusing to record')
    return ident


def record_baseline(files: list[str], jobs: int, timeout: int, force: bool = False) -> int:
    verify_pin()
    oracle = pinned_oracle()
    ident = {'emacs': oracle['resolved'], 'version': oracle['version'], 'sha256': oracle['sha256'],
             'revision': oracle['revision']}
    binary = oracle['path']
    BASELINE.mkdir(exist_ok=True)

    harness_sha = hashlib.sha256(HARNESS.read_bytes()).hexdigest()

    def one(rel):
        out = baseline_path(rel)
        old = json.loads(out.read_text()) if out.exists() and not force else {}
        # Keep regimes already recorded from the same file and GNU binary; the
        # harness-fidelity regime additionally needs the same harness.
        same_gnu = old.get('blob') == blob_of(rel) and old.get('gnu') == ident
        reusable = {'real': same_gnu, 'compiled': same_gnu,
                    'mini': same_gnu and old.get('harness_sha256') == harness_sha}
        record = {'schema': 1, 'file': rel, 'blob': blob_of(rel), 'gnu': ident, 'harness_sha256': harness_sha}
        with tempfile.TemporaryDirectory(prefix='gts-gnu-') as tmp:
            scratch = pathlib.Path(tmp)
            plan = extract(rel, scratch, binary)
            record.update(toplevel_deftests=plan['toplevel_deftests'], read_error=plan['read_error'])
            for regime in ('real', 'compiled', 'mini'):
                if reusable[regime] and regime in old:
                    record[regime] = old[regime]
                else:
                    record[regime] = gnu_run(regime, rel, scratch, timeout, plan, binary)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(record, indent=1, sort_keys=True) + '\n')
        return rel, len(record['real']['results']), record['real']['complete'] and record['compiled']['complete']

    before = lane_files()
    with ScratchTree(), concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        for rel, n, complete in pool.map(one, files):
            print(f'baseline {rel}: {n} GNU results{"" if complete else " (INCOMPLETE)"}', flush=True)
    verify_pin()
    stray = lane_files() - before
    if stray:
        # A GNU test resolved a path against this lane's own files and wrote
        # there: the baseline did not run the way `make check' runs.
        raise Refusal(4, 'GNU tests wrote into gnu-tests/ during the baseline: ' + ', '.join(sorted(stray)[:20]))
    return 0


def load_member_evidence(path: pathlib.Path | None = None) -> dict:
    """file::test -> verified run-local probe evidence."""
    sys.path.insert(0, str(HERE / 'tools'))
    from member_evidence import load
    try:
        return load(ROOT, path)
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        raise Refusal(3, f'subject evidence refused: {error}') from error


def lane_files() -> set[str]:
    skip = {'baseline', 'out', 'test', '__pycache__'}
    return {str(p.relative_to(HERE)) for p in HERE.rglob('*')
            if not skip.intersection(p.relative_to(HERE).parts)}


def pinned_gnu_sha256(reference: pathlib.Path = REFERENCE) -> str:
    """The executable digest of the pinned GNU (parity-reference.toml)."""
    try:
        with open(reference, 'rb') as stream:
            digest = tomllib.load(stream).get('executable_sha256', '')
    except (OSError, tomllib.TOMLDecodeError) as e:
        raise Refusal(3, f'cannot read the pinned GNU digest from {reference}: {e}; no baseline can be trusted') from None
    if not re.fullmatch(r'[0-9a-f]{64}', digest):
        raise Refusal(3, f'{reference}: executable_sha256 {digest!r} is not a sha256')
    return digest


RERECORD_COMMAND = 'ROMACS_GNU_ORACLE=<pinned GNU emacs> python3 gnu-tests/run.py baseline --all --force'


def baseline_provenance_problem(rel: str, record: dict, expected: str) -> str | None:
    """Why RECORD may not be used as GNU's answer at the pin, or None.

    Fail-closed: a baseline that names no binary, or any binary whose digest
    is not the pin's, is refused. All 565 baselines committed before this
    check were recorded by /usr/bin/emacs-32.0.50; they are refused until they
    are deliberately re-recorded (it moves the reduced-harness count).
    """
    gnu = record.get('gnu') or {}
    recorded = gnu.get('sha256')
    if recorded == expected:
        return None
    return (f'GNU baseline gnu-tests/baseline/{rel}.json was recorded by {gnu.get("emacs") or "<no binary named>"} '
            f'(sha256 {recorded or "<none>"}), not by the pinned GNU (expected sha256 {expected}, '
            f'parity-reference.toml executable_sha256). A baseline from another binary is not GNU\'s answer at '
            f'the pin, so it is refused, never used. Re-recording is its own explicit, deliberate step '
            f'(it changes what the suite counts): {RERECORD_COMMAND}')


def load_baseline(rel: str) -> dict:
    path = baseline_path(rel)
    if not path.exists():
        raise Refusal(3, f'no GNU baseline for {rel}; record it: run.py baseline {rel}')
    record = json.loads(path.read_text())
    problem = baseline_provenance_problem(rel, record, pinned_gnu_sha256())
    if problem:
        raise Refusal(3, problem)
    if record['blob'] != blob_of(rel):
        raise Refusal(3, f'GNU baseline for {rel} was recorded from blob {record["blob"]}, '
                         f'the file is {blob_of(rel)}; re-record it')
    if record['harness_sha256'] != hashlib.sha256(HARNESS.read_bytes()).hexdigest():
        raise Refusal(3, f'GNU baseline for {rel} measured a different harness; re-record it '
                         '(the fidelity check must describe the harness that ran)')
    if 'compiled' not in record:
        raise Refusal(3, f'GNU baseline for {rel} lacks the byte-compiled regime; run.py baseline {rel}')
    return record


# --------------------------------------------------------------------------
# romacs

def load_path() -> list[str]:
    """GNU's installed load-path: the Lisp root, then the immediate
    subdirectories named by its generated subdirs.el (build-aux/update-subdirs):
    reverse name order, obsolete last, term and VCS/backup names omitted."""
    lisp = ROOT / 'gnu-lisp'
    names = sorted(p.name for p in lisp.iterdir() if p.is_dir()
                   and p.name not in ('term', 'RCS', 'CVS', 'Old')
                   and not p.name.startswith('=') and not p.name.endswith(('~', '.orig', '.rej')))
    ordered = [n for n in reversed(names) if n != 'obsolete'] + (['obsolete'] if 'obsolete' in names else [])
    return [str(lisp)] + [str(lisp / name) for name in ordered]


def build_adapter() -> pathlib.Path:
    target = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
    p = subprocess.run(['cargo', 'build', '--offline', '--release', '-p', 'romacs-gnu-tests'],
                       cwd=ROOT, capture_output=True, text=True)
    if p.returncode != 0:
        raise Refusal(4, f'cargo build of romacs-gnu-ert failed (exit {p.returncode}):\n{p.stderr[-3000:]}')
    return target / 'release' / 'romacs-gnu-ert'


def run_romacs(binary: pathlib.Path, rel: str, plan: dict, scratch: pathlib.Path,
               test_timeout: float, setup_timeout: float, log, extra_args: tuple = (),
               bootstrap: str = 'standard') -> dict:
    """Run every test of one file, resuming past a hung or crashing test.

    Returns {'events': [...], 'results': {name: result}, 'fatal': str|None}.
    """
    results: dict[str, dict] = {}
    skip: list[str] = []
    events: list[dict] = []
    fatal = None
    for attempt in range(1, 10_000):
        cmd = [str(binary), '--harness', str(HARNESS), '--audit', str(HARNESS_AUDIT), '--plan', plan['path']]
        for d in load_path():
            cmd += ['--load-path', d]
        for d in [str((RUN_TESTS / rel).parent)]:
            cmd += ['--load-path', d]
        if bootstrap == 'ert':
            cmd += ['--bootstrap-regime', 'gnu-ert', '--load-path', str(ROOT / 'rlisp')]
            for link in ERT_SOURCE_BOOTSTRAP:
                cmd += ['--preload', str(ROOT / link)]
        else:
            for link in BOOTSTRAP_CHAIN:
                cmd += ['--preload', str(ROOT / 'gnu-lisp' / f'{link}.el')]
        for name in skip:
            cmd += ['--skip-test', name]
        cmd += list(extra_args)  # the runner's own tests only
        env = dict(os.environ, HOME=str(scratch), EMACS_TEST_DIRECTORY=str(RUN_TESTS),
                   ROMACS_SOURCE_DIRECTORY=str(ROOT))
        proc = subprocess.Popen(cmd, cwd=(RUN_TESTS / rel).parent, env=env, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True, errors='replace')
        lines: queue.Queue = queue.Queue()

        def pump(stream=proc.stdout):
            for line in stream:
                lines.put(line)
            lines.put(None)
        threading.Thread(target=pump, daemon=True).start()
        stderr_chunks: list[str] = []
        threading.Thread(target=lambda: stderr_chunks.append(proc.stderr.read()), daemon=True).start()
        current = None
        done = False
        deadline = setup_timeout
        while True:
            try:
                line = lines.get(timeout=deadline)
            except queue.Empty:
                proc.kill()
                break
            if line is None:
                break
            log.write(line)
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            events.append(event)
            kind = event.get('event')
            if kind == 'start':
                current, deadline = event['test'], test_timeout
            elif kind == 'result':
                results[event['test']] = event
                current, deadline = None, test_timeout
            elif kind == 'done':
                done = True
        proc.wait()
        log.write(f'# attempt {attempt}: exit {proc.returncode}\n')
        if stderr_chunks and stderr_chunks[0]:
            log.write('# stderr: ' + stderr_chunks[0][-4000:].replace('\n', '\n# ') + '\n')
        if done and proc.returncode == 0:
            break
        if current is not None:
            panic = next((e for e in reversed(events) if e.get('event') == 'panic'), None)
            outcome = 'panic' if panic and panic.get('test') == current else 'timeout'
            results[current] = {'event': 'result', 'test': current, 'outcome': outcome, 'expected': None,
                                'detail': (panic or {}).get('detail') or
                                f'no result within {test_timeout}s; the process was killed'}
            skip.append(current)
            continue
        fatal = (f'adapter exited {proc.returncode} before running tests'
                 + (f': {events[-1]}' if events else '')
                 + (f'; stderr: {stderr_chunks[0][-800:]}' if stderr_chunks and stderr_chunks[0] else ''))
        break
    return {'events': events, 'results': results, 'fatal': fatal}


# --------------------------------------------------------------------------
# Classification (pure; unit-tested)

def missing_from(detail: str | None) -> str | None:
    """Name what is missing, mechanically, or None when it cannot be named.

    A void-function/void-variable anywhere in the failure (including the
    condition a should-error caught) names the symbol; a VM refusal that says
    it does not support something names that, without byte offsets so equal
    gaps aggregate.
    """
    if not detail:
        return None
    m = re.search(r'\(void-function ([^\s()]+)\)', detail) or re.match(r'^void-function ([^\s()]+)$', detail.strip())
    if m:
        return m.group(1)
    m = re.search(r'\(void-variable ([^\s()]+)\)', detail) or re.match(r'^void-variable ([^\s()]+)$', detail.strip())
    if m:
        return f'variable {m.group(1)}'
    if UNSUPPORTED.search(detail):
        return re.sub(r' at byte \d+', '', detail.strip())[:160]
    return None


ERT_ONLY = re.compile(r'^(ert-|ert--)')


def vacuous_catches(romacs: dict, fidelity: dict | None, evidence: dict | None) -> list[str] | None:
    """The should-error catches by which absent machinery satisfied a romacs test.

    romacs['caught'] lists each should-error whose condition was void-function,
    void-variable or invalid-function and not one the test asked for
    (gnu-tests/harness/romacs-mini-ert-audit.el). A catch is excused only when
    pinned GNU caught the same condition and datum with matching multiplicity.
    Legacy baselines without a full audit use historical evidence until a
    fresh pinned GNU audit is obtained by refresh_catch_audit. Returns None
    when romacs reported no audit at all: then nobody knows.
    """
    caught = romacs.get('caught')
    if caught is None:
        return None
    gnu_caught = set((evidence or {}).get('gnu_caught') or [])
    gnu_void = set((fidelity or {}).get('vacuous_void_function') or [])
    # A fresh audit is stronger than historical type-only evidence. Consume
    # exact pairs: one native catch cannot excuse two VM catches, nor can a
    # native missing setter excuse a VM missing subject with the same type.
    audited = fidelity is not None and 'caught' in fidelity
    native = collections.Counter(tuple(pair) for pair in (fidelity or {}).get('caught', []))
    out = []
    for condition, datum in caught:
        if condition not in ABSENT_MACHINERY:
            continue
        pair = (condition, datum)
        if audited and datum and native[pair]:
            native[pair] -= 1
            continue
        if not audited and (condition in gnu_caught or (condition == 'void-function' and datum and datum in gnu_void)):
            continue
        out.append(f'({condition} {datum})' if datum else f'({condition})')
    return out


def refresh_catch_audit(rel: str, baseline: dict, plan: dict, vm_run: dict,
                        scratch: pathlib.Path, timeout: float) -> dict:
    """Recheck suspect passes in pinned GNU; never edit the frozen baseline.

    Old mini baselines only audited void-function, so lack of a void-variable
    row is not evidence that GNU caught a different condition. Require a
    complete fresh run and unchanged fidelity verdict before using its audit.
    """
    mini = baseline['mini']['results']
    suspects = [name for name, result in vm_run['results'].items()
                if result['outcome'] == 'passed' and result.get('caught')
                and 'caught' not in mini.get(name, {})
                and any(baseline.get(regime, {}).get('results', {}).get(name, {}).get('outcome')
                        not in (None, 'not-recorded') for regime in ('real', 'compiled'))]
    if not suspects:
        return baseline
    oracle = pinned_oracle()
    fresh = gnu_run('mini', rel, scratch, timeout, plan, oracle['path'])
    (scratch / 'native-catch-audit.json').write_text(json.dumps(fresh, indent=1) + '\n')
    if (fresh['exit'] != 0 or not fresh['complete'] or fresh['timed_out'] or fresh['garbled_lines']):
        raise Refusal(4, f'{rel}: incomplete pinned GNU catch audit; no pass may be excused')
    results = dict(mini)
    for name in suspects:
        observed, recorded = fresh['results'].get(name), mini.get(name)
        if (not observed or not recorded or 'caught' not in observed
                or observed['outcome'] != recorded['outcome']):
            raise Refusal(4, f'{rel}::{name}: pinned GNU catch audit missing or disagrees with baseline')
        results[name] = dict(recorded, caught=observed['caught'])
    return dict(baseline, mini=dict(baseline['mini'], results=results))


def classify(name: str, gnu: dict | None, fidelity: dict | None, romacs: dict | None,
             not_run_reason: str | None, exclusion: dict | None, gnu_compiled: dict | None = None,
             evidence: dict | None = None) -> dict:
    """One row per test.  gnu/fidelity/romacs are result dicts or None.

    gnu is GNU loading the source (the regime romacs's per-form evaluation
    mirrors); gnu_compiled is GNU running the byte-compiled file, as `make
    check' does.  GNU can disagree with itself across the two (bytecode.c and
    fns.c signal different data for the same call); matching EITHER regime's
    as-expected outcome is parity.
    """
    regimes = [g for g in (gnu, gnu_compiled) if g and g['outcome'] != 'not-recorded']
    row = {'harness': 'romacs-mini-ert', 'test': name, 'gnu': gnu and gnu['outcome'], 'gnu_compiled': gnu_compiled and gnu_compiled['outcome'],
           'gnu_as_expected': any(g.get('as_expected') for g in regimes),
           'harness_in_gnu': fidelity and fidelity['outcome'],
           'romacs': romacs and romacs['outcome'], 'detail': romacs and romacs.get('detail'),
           'verdict': None, 'category': None, 'missing': None, 'reason': None, 'excluded_by': None,
           'vacuous_catch': None}
    if gnu is None:
        raise ValueError(f'{name}: not a GNU test; extras are reported separately')
    if not regimes:
        row['verdict'] = ('not-yet-runnable' if romacs is None else
                          {'passed': 'passed', 'failed': 'failed', 'skipped': 'skipped'}.get(romacs['outcome'], 'errored'))
        if row['verdict'] == 'passed':
            row['verdict'] = 'failed'  # no GNU verdict to agree with
        row['category'] = 'gnu-not-recorded'
        row['reason'] = 'GNU\'s baseline run never reported this test'
        return row
    if romacs is None:
        row['verdict'] = 'not-yet-runnable'
        row['reason'] = not_run_reason or 'romacs never registered this test'
        row['missing'] = missing_from(not_run_reason)
    elif fidelity is None or fidelity['outcome'] not in {g['outcome'] for g in regimes}:
        # Under GNU itself the reduced harness does not reproduce real ERT for
        # this test, so a romacs verdict here would measure the harness.
        row['verdict'] = 'not-yet-runnable'
        harness_missing = missing_from(fidelity and fidelity.get('detail'))
        row['missing'] = f'harness: {harness_missing or "real ert.el"}'
        row['reason'] = (f'romacs-mini-ert under GNU gives {fidelity and fidelity["outcome"]!r} '
                         f'where real ERT gives {gnu["outcome"]!r}')
    else:
        r = romacs['outcome']
        g = '/'.join(sorted({x['outcome'] for x in regimes}))
        if any(r == x['outcome'] and x.get('as_expected') and x['outcome'] in ('passed', 'failed') for x in regimes):
            row['verdict'] = 'passed'
            vacuous = vacuous_catches(romacs, fidelity, evidence)
            row['vacuous_catch'] = vacuous
            if vacuous is None:
                row['verdict'], row['category'] = 'failed', 'unaudited-pass'
                row['reason'] = ('romacs reported no should-error audit for this test; a pass whose caught '
                                 'conditions nobody saw is not counted')
                return row
            if vacuous:
                # GNU's guard (gnu-run.el vacuous_void_function), applied to
                # the romacs rows that are counted: absent machinery satisfied
                # a should-error where pinned GNU caught a real condition.
                row['verdict'], row['category'] = 'failed', 'vacuous-pass'
                row['missing'] = missing_from(' '.join(vacuous))
                row['reason'] = ('should-error satisfied by absent machinery: ' + ', '.join(vacuous[:8])
                                 + '; pinned GNU caught a real condition there')
                return row
        else:
            row['verdict'] = {'passed': 'failed', 'failed': 'failed', 'skipped': 'skipped'}.get(r, 'errored')
            row['missing'] = missing_from(romacs.get('detail'))
            row['reason'] = f'GNU {g}, romacs {r}'
    if row['verdict'] == 'passed':
        if exclusion:
            raise Refusal(2, f'{exclusion["file"]}::{name} PASSES on romacs but EXCLUSIONS.toml excludes it '
                             f'(divergence {exclusion["divergence"]}); delete the row')
        return row
    if exclusion:
        # Only a row whose GNU side was re-checked against the pinned source,
        # and whose ledger entry is not known-wrong, counts as a divergence.
        row['category'] = exclusion['category'] if exclusion.get('verified') else 'excluded-unverified-row'
        row['excluded_by'] = exclusion['divergence']
        row['reason'] = '; '.join(exclusion.get('unverified_because') or []) or row['reason']
    elif not row['gnu_as_expected']:
        row['category'] = 'gnu-fails-too'
    elif all(x['outcome'] == 'skipped' for x in regimes):
        row['category'] = 'gnu-skips-too'
    elif row['missing'] and not ERT_ONLY.match(row['missing']):
        row['category'] = 'not-yet-implemented'
    elif row['missing']:
        row['category'] = 'not-yet-implemented'
        row['missing'] = f'harness: {row["missing"]}'
    else:
        row['category'] = 'failing'
    return row


def not_run_reason(plan: dict, run: dict | None, name: str) -> str:
    if run is None:
        return 'file not run'
    if run['fatal']:
        return run['fatal']
    form = next((f for f in plan['forms'] if f.get('deftest') == name), None)
    if form is None:
        return 'defined by a generated or nested form romacs did not register'
    event = next((e for e in run['events'] if e.get('event') == 'form' and e.get('ordinal') == form['ordinal']), None)
    if event is None:
        return 'its defining form was never evaluated'
    if not event['ok']:
        return event.get('error') or 'its defining form failed'
    if plan.get('shorthands'):
        return ('its form ran, but GNU reads this file with read-symbol-shorthands '
                f'{plan["shorthands"]} and romacs does not, so the test registered under '
                'its unexpanded name (void-variable read-symbol-shorthands)')
    return 'registered by its form but never reported'


def file_load_facts(plan: dict, run: dict | None) -> dict:
    """Did romacs evaluate every top-level form of the file without error?

    The reduced harness evaluates form by form and keeps going after a failed
    `require`, so a test can register and pass while its subject never
    loaded. GNU's `load` would have aborted the file there.
    """
    if run is None or run.get('fatal'):
        return {'file_loaded_clean': False, 'failed_forms': []}
    status = {}
    for event in run['events']:
        if event.get('event') == 'form':
            status[event.get('ordinal')] = bool(event.get('ok'))
    failed = sorted(o for o, ok in status.items() if not ok)
    unreported = [f['ordinal'] for f in plan['forms'] if f['ordinal'] not in status]
    return {'file_loaded_clean': not failed and not unreported, 'failed_forms': failed[:50],
            'unreported_forms': unreported[:50]}


def classify_file(rel: str, baseline: dict, plan: dict, run: dict | None, ledger: dict,
                  evidence: dict | None = None) -> tuple[list, list]:
    rows, extras = [], []
    facts = file_load_facts(plan, run)
    evidence = evidence or {}
    gnu = dict(baseline['real']['results'])
    for form in plan['forms']:
        if form.get('deftest') and form['deftest'] not in gnu:
            gnu[form['deftest']] = NOT_RECORDED
    mini = baseline['mini']['results']
    compiled = baseline.get('compiled', {}).get('results', {})
    romacs = run['results'] if run else {}
    for name, g in gnu.items():
        r = romacs.get(name)
        row = classify(name, g, mini.get(name), r, None if r else not_run_reason(plan, run, name),
                       ledger.get((rel, name)), compiled.get(name), evidence.get(f'{rel}::{name}'))
        row['file'] = rel
        row.update(facts)
        rows.append(row)
    for name in romacs:
        if name not in gnu:
            extras.append({'file': rel, 'test': name, 'romacs': romacs[name]['outcome']})
    for (f, t), ex in ledger.items():
        if f == rel and t not in gnu:
            raise Refusal(2, f'EXCLUSIONS.toml names {rel}::{t}, which GNU does not define')
    return rows, extras


def summarize(rows: list[dict], files: list[str], extra: dict) -> dict:
    def count(key, values):
        return {v: sum(1 for r in rows if r[key] == v) for v in values}
    gnu_expected = [r for r in rows if r['gnu_as_expected'] and r['gnu'] in ('passed', 'failed')]
    summary = {
        'headline': {
            'gnu_tests': len(rows),
            'gnu_tests_gnu_gives_expected_result': len(gnu_expected),
            'passed_on_romacs': sum(1 for r in rows if r['verdict'] == 'passed'),
            'runnable_on_romacs': sum(1 for r in rows if r['verdict'] != 'not-yet-runnable'),
            'make_check_scope': {
                'gnu_tests': sum(1 for r in rows if in_make_check(r['file'])),
                'passed_on_romacs': sum(1 for r in rows if in_make_check(r['file']) and r['verdict'] == 'passed'),
            },
        },
        'verdicts': count('verdict', VERDICTS),
        'categories': count('category', CATEGORIES),
        'files': len(files),
    }
    missing: dict[str, int] = {}
    for r in rows:
        if r['category'] == 'not-yet-implemented':
            missing[r['missing']] = missing.get(r['missing'], 0) + 1
    summary['top_missing'] = sorted(missing.items(), key=lambda kv: (-kv[1], kv[0]))[:60]
    summary.update(extra)
    if sum(summary['verdicts'].values()) != len(rows):
        raise AssertionError('a row has no verdict')
    if sum(summary['categories'].values()) != len(rows) - summary['verdicts']['passed']:
        raise AssertionError('a non-passing row has no category')
    return summary


# --------------------------------------------------------------------------

def harness_defects(real: dict, mini: dict) -> list[dict]:
    """Compare the full union: missing or extra tests are defects too."""
    if not real:
        raise Refusal(4, 'harness fidelity: zero tests in real ERT')
    def outcome(row):
        value = (row or {}).get('outcome')
        return 'failed' if value == 'errored' else value
    def expected(row):
        row = row or {}
        if row.get('as_expected') is not None:
            return row['as_expected']
        # Mini records only these three supported expected-result types.
        wanted = row.get('expected')
        if wanted == 't':
            return True
        if wanted in (':passed', ':failed'):
            return outcome(row) == wanted[1:]
        return None
    return [{'test': name, 'real': real.get(name), 'mini': mini.get(name),
             'category': 'harness-defect'}
            for name in sorted(real.keys() | mini.keys())
            if outcome(real.get(name)) != outcome(mini.get(name))
            or expected(real.get(name)) != expected(mini.get(name))
            or real.get(name, {}).get('vacuous_void_function', [])
            != mini.get(name, {}).get('vacuous_void_function', [])]


def scoreboard_rows(rel, reference, results, engine, ledger):
    rows = []
    for name in sorted(reference.keys() | results.keys()):
        result, ref = results.get(name), reference.get(name)
        passed = bool(result and ref and result == ref and result['as_expected']
                      and result['outcome'] != 'skipped')
        unmatched = list((collections.Counter((result or {}).get('vacuous_void_function', []))
                          - collections.Counter((ref or {}).get('vacuous_void_function', []))).elements())
        exclusion = ledger.get((rel, name))
        if engine == 'romacs' and exclusion and passed:
            raise Refusal(2, f'{rel}::{name} PASSES but EXCLUSIONS.toml excludes it')
        rows.append({'file': rel, 'test': name, 'harness': 'gnu-ert', 'engine': engine,
                     'outcome': result and result['outcome'],
                     'as_expected': result and result['as_expected'], 'gnu': ref,
                     'vacuous_void_function': result and result.get('vacuous_void_function', []),
                     'unmatched_void_function': unmatched,
                     'excluded_by': exclusion,
                     'verdict': 'passed' if passed else 'failed'})
    for file, name in ledger:
        if file == rel and name not in reference:
            raise Refusal(2, f'EXCLUSIONS.toml names {rel}::{name}, which GNU does not register')
    return rows


def verify_real_lisp(pin):
    command = [sys.executable, str(ROOT / 'tools/lib/lisp_oracle.py'), 'check',
               '--root', str(ROOT), '--gnu', str(GNU_MIRROR), '--pin', pin]
    try:
        result = subprocess.run(command, capture_output=True, text=True)
    except OSError as error:
        raise Refusal(3, f'cannot verify real ERT Lisp tree: {error}') from error
    if result.returncode:
        raise Refusal(3, f'real ERT Lisp tree validation failed: {result.stdout}{result.stderr}')


def cmd_scoreboard(args) -> int:
    """Real ERT execution; no mini classification or stale baseline can bless it."""
    from ert_report import ReportError, parse_report, run_native
    sys.path.insert(0, str(ROOT / 'tools/lib'))
    import gnu_oracle
    p = verify_pin()
    verify_real_lisp(p['gnu_commit'])
    # The oracle's binary identity, not the checkout name, binds GNU evidence.
    oracle_binary = os.environ.get('ROMACS_GNU_ORACLE') or os.environ.get('ROMACS_FORCE_ORACLE_PATH')
    ident = gnu_oracle.require(oracle_binary)
    files = (sorted(str(f.relative_to(TESTS)) for f in (TESTS / 'lisp').rglob('*.el')
                    if in_make_check(str(f.relative_to(TESTS)))) if args.all
             else [normalize(f) for f in args.files])
    if not files or args.jobs < 1:
        raise Refusal(2, 'name test files, or --all; jobs must be positive')
    out = pathlib.Path(args.out or (HERE / 'out' / time.strftime('%Y%m%d-%H%M%S'))).resolve()
    if out.exists() and any(out.iterdir()):
        raise Refusal(2, 'scoreboard output directory must be empty')
    (out / 'log').mkdir(parents=True, exist_ok=True)
    ledger = load_ledger(LEDGER)
    engine = 'gnu' if args.harness_fidelity else args.engine
    binary = (pathlib.Path(args.bin) if args.bin else build_adapter()) if engine == 'romacs' else None
    ert = ROOT / 'gnu-lisp' / 'emacs-lisp' / 'ert.el'
    rows, defects, reports, errors, executions = [], [], {}, {}, {}
    with ScratchTree(), tempfile.TemporaryDirectory(prefix='gts-real-') as tmp:
        scratch = pathlib.Path(tmp)

        def one(rel):
            home = scratch / rel.replace('/', '__')
            home.mkdir()
            target = RUN_TESTS / rel
            logs = []
            failed_engine = 'gnu'
            try:
                reference = run_native(ident['path'], target, None, ert,
                                       [str(target.parent)], home, args.setup_timeout,
                                       environment=gnu_env(home), selector=getattr(args, 'selector', 't'))
                logs.append(reference['output'])
                report = reference
                if engine == 'romacs':
                    failed_engine = 'romacs'
                    plan = extract(rel, home, ident['path'])
                    cmd = [str(binary), '--harness-kind', 'real', '--harness', str(ert),
                           '--plan', plan['path'], '--selector', getattr(args, 'selector', 't')]
                    if getattr(args, 'ert_adapter', 'none') != 'none':
                        cmd += ['--ert-adapter', args.ert_adapter]
                    for directory in load_path() + [str(ROOT / 'rlisp'), str(target.parent)]:
                        cmd += ['--load-path', directory]
                    if getattr(args, 'bootstrap', 'standard') == 'ordered':
                        cmd += ['--measurement-regime', 'retained-bindings-prefix']
                    if getattr(args, 'bootstrap', 'standard') == 'ert':
                        cmd += ['--bootstrap-regime', 'gnu-ert']
                    for link in source_bootstrap_chain(args):
                        path = ROOT / link if getattr(args, 'bootstrap', 'standard') == 'ert' else ROOT / 'gnu-lisp' / f'{link}.el'
                        cmd += ['--preload', str(path)]
                    proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                          text=True, errors='replace',
                                          env=dict(gnu_env(home), ROMACS_SOURCE_DIRECTORY=str(ROOT)),
                                          timeout=args.setup_timeout, cwd=target.parent)
                    logs.append(proc.stdout)
                    report = parse_report(proc.stdout, proc.returncode)
                mini = None
                if args.harness_fidelity:
                    plan = extract(rel, home, ident['path'])
                    mini = gnu_run('mini', rel, home, args.setup_timeout, plan, ident['path'])
                    if not mini['complete'] or mini['exit'] != 0 or mini['timed_out'] or mini['garbled_lines']:
                        raise ReportError('incomplete mini harness fidelity execution')
                return rel, reference, report, mini, None
            except (ReportError, Refusal, OSError, subprocess.TimeoutExpired) as error:
                diagnostic = getattr(error, 'output', '') or ''
                if diagnostic not in logs:
                    logs.append(diagnostic)
                evidence = getattr(error, 'evidence', None)
                if evidence is None and isinstance(error, subprocess.TimeoutExpired):
                    try:
                        parse_report((error.output or b'').decode(errors='replace') if isinstance(error.output, bytes) else error.output or '', -1)
                    except ReportError as incomplete:
                        evidence = incomplete.evidence
                return rel, None, dict(evidence or ReportError(str(error)).evidence,
                                       engine=failed_engine, complete=False), None, str(error)
            finally:
                (out / 'log' / (rel.replace('/', '__') + '.log')).write_text(
                    '\n'.join(x.decode(errors='replace') if isinstance(x, bytes) else x for x in logs))

        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            for rel, reference, report, mini, error in pool.map(one, files):
                executions[rel] = dict({key: report[key] for key in ('selected', 'started', 'terminal', 'execution', 'complete') if key in report},
                                       engine=report.get('engine', engine))
                if error:
                    errors[rel] = error
                    continue
                reports[rel] = report['summary']
                if mini is not None:
                    defects.extend(dict(d, file=rel) for d in harness_defects(reference['results'], mini['results']))
                try:
                    rows.extend(scoreboard_rows(rel, reference['results'], report['results'], engine, ledger))
                except Refusal as error:
                    errors[rel] = str(error)
    verify_pin()
    summary = {'harness': 'gnu-ert', 'is_gnu_ert': True, 'engine': engine,
               'is_unmodified_gnu_ert': getattr(args, 'ert_adapter', 'none') == 'none',
               'ert_adapter': {'name': getattr(args, 'ert_adapter', 'none'),
                               'sha256': hashlib.sha256((ROOT / 'rlisp/romacs-ert-batch.el').read_bytes()).hexdigest()
                               if getattr(args, 'ert_adapter', 'none') == 'romacs-batch' else None,
                               'condition_format': 'native-prin1' if getattr(args, 'ert_adapter', 'none') == 'romacs-batch' else 'gnu-pp',
                               'automatic_window_isolation': getattr(args, 'ert_adapter', 'none') == 'none'},
               'pin': p, 'gnu': ident, 'files': len(files), 'completed_files': len(reports),
               'selector': getattr(args, 'selector', 't'),
               'bootstrap_regime': ('MEASUREMENT REGIME: retained failed bindings prefix'
                                    if getattr(args, 'bootstrap', 'standard') == 'ordered'
                                    else 'gnu-ert' if getattr(args, 'bootstrap', 'standard') == 'ert' else 'standard'),
               'bootstrap_chain': source_bootstrap_chain(args) if engine == 'romacs' else [],
               'load_strategy': 'GNU pinned install; romacs pristine source tree (recursive source load path)',
               'scope': 'test/lisp, GNU Makefile directory pruning; empty files fail closed',
               'romacs_commit': git_or_none('rev-parse', 'HEAD'),
               'adapter_sha256': hashlib.sha256(binary.read_bytes()).hexdigest() if binary else None,
               'excluded_tests': sum(bool(row['excluded_by']) for row in rows),
               'vacuous_passes': sum(bool(row['unmatched_void_function']) for row in rows),
               'ert_summaries': reports, 'execution_reports': executions, 'infrastructure_errors': errors,
               'harness_defects': defects, 'harness_fidelity': args.harness_fidelity,
               'ert_totals': {key: sum(s[key] for s in reports.values())
                              for key in ('passed', 'failed', 'skipped', 'total')}}
    (out / 'results.jsonl').write_text(''.join(json.dumps(row, sort_keys=True) + '\n' for row in rows))
    (out / 'summary.json').write_text(json.dumps(summary, indent=1, sort_keys=True) + '\n')
    print(json.dumps(summary, sort_keys=True))
    if errors or len(reports) != len(files) or not rows:
        return 4
    if args.harness_fidelity:
        return 1 if defects else 0
    return 0 if all(row['verdict'] == 'passed' for row in rows) else 1


def cmd_run(args) -> int:
    if getattr(args, 'ert_adapter', 'none') != 'none' and (
            args.harness != 'real' or args.engine != 'romacs' or args.harness_fidelity):
        raise Refusal(2, '--ert-adapter requires --harness real --engine romacs without --harness-fidelity')
    if getattr(args, 'bootstrap', 'standard') == 'ordered' and (
            args.harness != 'real' or args.engine != 'romacs' or args.harness_fidelity):
        raise Refusal(2, f'--bootstrap {args.bootstrap} requires --harness real --engine romacs')
    if getattr(args, 'bootstrap', 'standard') == 'ert' and (
            args.engine != 'romacs' or args.harness_fidelity):
        raise Refusal(2, '--bootstrap ert requires --engine romacs without --harness-fidelity')
    if getattr(args, 'selector', 't') != 't' and args.harness != 'real':
        raise Refusal(2, '--selector requires --harness real')
    if args.harness == 'real' or args.harness_fidelity:
        return cmd_scoreboard(args)
    if args.engine != 'romacs':
        raise Refusal(2, '--engine gnu requires --harness real or --harness-fidelity')
    p = verify_pin()
    files = suite_files() if args.all else [normalize(f) for f in args.files]
    if not files:
        raise Refusal(2, 'name test files, or --all')
    out = pathlib.Path(args.out or (HERE / 'out' / time.strftime('%Y%m%d-%H%M%S'))).resolve()
    (out / 'log').mkdir(parents=True, exist_ok=True)
    baselines = {rel: load_baseline(rel) for rel in files}
    binary = pathlib.Path(args.bin) if args.bin else build_adapter()
    tmp = tempfile.TemporaryDirectory(prefix='gts-run-')
    scratch = pathlib.Path(tmp.name)
    tree = ScratchTree()
    tree.__enter__()
    plans = {}
    for rel in files:
        plans[rel] = extract(rel, scratch)
    ledger = load_ledger(LEDGER, plans)
    evidence = load_member_evidence()

    def one(rel):
        home = scratch / ('home-' + rel.replace('/', '__'))
        home.mkdir()
        with open(out / 'log' / (rel.replace('/', '__') + '.log'), 'w') as log:
            result = run_romacs(binary, rel, plans[rel], home, args.test_timeout, args.setup_timeout, log,
                                bootstrap=getattr(args, 'bootstrap', 'standard'))
        try:
            baselines[rel] = refresh_catch_audit(rel, baselines[rel], plans[rel], result, home, args.setup_timeout)
        finally:
            audit = home / 'native-catch-audit.json'
            if audit.exists():
                shutil.copyfile(audit, out / 'log' / (rel.replace('/', '__') + '.native-catch-audit.json'))
        return rel, result

    runs = {}
    try:
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            for rel, run in pool.map(one, files):
                runs[rel] = run
    finally:
        tree.__exit__(None, None, None)
    verify_pin()  # the run cannot have touched the import
    rows, extras = [], []
    for rel in files:
        r, e = classify_file(rel, baselines[rel], plans[rel], runs[rel], ledger, evidence)
        rows += r
        extras += e
    chain = next((e['entries'] for run in runs.values() for e in run['events'] if e.get('event') == 'chain'), None)
    summary = summarize(rows, files, {
        'harness': {'kind': 'reduced', 'name': 'romacs-mini-ert', 'file': str(HARNESS.relative_to(ROOT)),
                    'is_gnu_ert': False,
                    'note': 'NOT GNU ert.el. Verdicts are only counted where this harness, run inside GNU, '
                            'reproduces real ERT\'s outcome (see harness_in_gnu per row).'},
        'pin': p, 'gnu': next(iter(baselines.values()))['gnu'],
        'adapter_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'romacs_commit': git_or_none('rev-parse', 'HEAD'),
        'romacs_tree_dirty': None if git_or_none('rev-parse', 'HEAD') is None else bool(git_or_none('status', '--porcelain')),
        'bootstrap_regime': 'gnu-ert' if getattr(args, 'bootstrap', 'standard') == 'ert' else 'standard',
        'bootstrap_chain': source_bootstrap_chain(args),
        'bootstrap_chain_reached': chain,
        'textual_ert_deftest_forms': sum((TESTS / rel).read_text(errors='replace').count('(ert-deftest')
                                         for rel in files if (TESTS / rel).is_file()),
        'gnu_baseline_incomplete_files': sorted(rel for rel in files
                                                if not (baselines[rel]['real']['complete']
                                                        and baselines[rel]['compiled']['complete'])),
        'exclusions': {
            'mechanically_validated_rows': sum(1 for r in rows if r['category'] in EXCLUSION_CATEGORIES),
            'unverified_row_debt': sum(1 for r in rows if r['category'] == 'excluded-unverified-row'),
            'note': 'A mechanically validated row cites GNU code whose bytes still match at the pin and a '
                    'ledger entry not listed in SUSPECT-DIVERGENCES.toml. That is citation identity, not '
                    'semantic proof: every row also needs human review of the cited code before it lands. '
                    'Unverified rows are debt, never parity evidence.'},
        'romacs_only_tests': extras,
        'per_file': {rel: {v: sum(1 for r in rows if r['file'] == rel and r['verdict'] == v) for v in VERDICTS}
                     for rel in files},
    })
    with open(out / 'results.jsonl', 'w') as f:
        for r in rows:
            f.write(json.dumps(r, sort_keys=True) + '\n')
    (out / 'summary.json').write_text(json.dumps(summary, indent=1, sort_keys=True) + '\n')
    h = summary['headline']
    print(f"harness: romacs-mini-ert (REDUCED, not GNU ert.el)")
    print(f"GNU tests {h['gnu_tests']}  (GNU gives the expected result for {h['gnu_tests_gnu_gives_expected_result']})")
    print(f"PASSED on romacs {h['passed_on_romacs']}  runnable {h['runnable_on_romacs']}")
    print('verdicts  ' + '  '.join(f'{k} {v}' for k, v in summary['verdicts'].items()))
    print('categories ' + '  '.join(f'{k} {v}' for k, v in summary['categories'].items()))
    print(f'wrote {out}/summary.json, results.jsonl, log/')
    tmp.cleanup()
    return 0 if summary['verdicts']['passed'] == len(rows) else 1


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    r = sub.add_parser('run')
    r.add_argument('files', nargs='*')
    r.add_argument('--all', action='store_true')
    r.add_argument('--jobs', type=int, default=4)
    r.add_argument('--out')
    r.add_argument('--harness', choices=('mini', 'real'), default='mini')
    r.add_argument('--bootstrap', choices=('standard', 'ordered', 'ert'), default='standard',
                   help='ert: strict GNU ERT source dependencies; ordered: retained-prefix measurement only')
    r.add_argument('--selector', default='t', help='GNU ERT selector as a Lisp expression; real harness only')
    r.add_argument('--ert-adapter', choices=('none', 'romacs-batch'), default='none',
                   help='romacs-batch: GNU ERT without precautionary window-layout isolation')
    r.add_argument('--harness-fidelity', action='store_true',
                   help='compare mini under native pinned GNU with genuine ERT; differences are defects')
    r.add_argument('--engine', choices=('romacs', 'gnu'), default='romacs',
                   help='real harness execution engine (GNU validates runner, not romacs parity)')
    r.add_argument('--bin', help='prebuilt romacs-gnu-ert (default: cargo build it)')
    r.add_argument('--test-timeout', type=float, default=60)
    r.add_argument('--setup-timeout', type=float, default=300)
    b = sub.add_parser('baseline')
    b.add_argument('files', nargs='*')
    b.add_argument('--all', action='store_true')
    b.add_argument('--jobs', type=int, default=4)
    b.add_argument('--timeout', type=int, default=600)
    b.add_argument('--force', action='store_true', help='re-record every regime, not only missing ones')
    c = sub.add_parser('check-ledger')
    c.add_argument('--ledger', default=str(LEDGER), help='validate another file (to demonstrate a refusal)')
    sub.add_parser('verify-pin')
    args = ap.parse_args(argv)
    try:
        if args.cmd == 'run':
            return cmd_run(args)
        if args.cmd == 'baseline':
            files = suite_files() if args.all else [normalize(f) for f in args.files]
            return record_baseline(files, args.jobs, args.timeout, args.force)
        if args.cmd == 'check-ledger':
            ledger = pathlib.Path(args.ledger).resolve()
            with tempfile.TemporaryDirectory(prefix='gts-ledger-') as tmp:
                rows = load_ledger(ledger)
                plans = {rel: extract(rel, pathlib.Path(tmp)) for rel in {f for f, _ in rows}}
                rows = load_ledger(ledger, plans)
            print(f'{ledger}: {len(rows)} valid row(s)')
            return 0
        if args.cmd == 'verify-pin':
            p = verify_pin()
            print(f'gnu-tests/test is GNU {p["gnu_commit"]}:test, tree {p["test_tree"]}, unmodified')
            return 0
    except Refusal as e:
        print(f'run.py: REFUSED: {e}', file=sys.stderr)
        return e.status
    return 2


if __name__ == '__main__':
    sys.exit(main())
