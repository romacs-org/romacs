#!/usr/bin/env python3
"""The parity ratchet: GNU-suite passes and Lisp-census loads are floors.

The gate's Rust suites say nothing about behaviour against GNU. The numbers the
project is judged by -- how many of GNU's own ERT tests pass (gnu-tests/run.py)
and how many of GNU's Lisp files load (scripts/lisp-census.py, WARM and COLD) --
were protected only by lanes choosing to re-measure. This makes them floors.

parity-ratchet.json (repo root) records, per unit, the exact MEMBERS behind each
number: per GNU test file the deftests that pass, per census mode the GNU files
that load. A count is the length of its member list.

    parity_ratchet.py check --unit UNIT [--gnu-ert BIN] [--census-bin BIN]
        Measure UNIT and compare it with its floor. UNIT is one of
          gnu-shard:K/N   GNU test files sorted, every Nth starting at K-1
          gnu-files:A,B   the named GNU test files
          census-warm     WARM census, GNU-parity files loaded
          census-cold     COLD census, GNU-parity files loaded
        Equal is PASS (exit 0). A floor member that no longer passes/loads is
        FAIL (exit 1), naming the unit, floor, measurement, delta and each lost
        member with its current verdict -- also when a gain elsewhere hides the
        loss in the count. A new member with nothing lost is FAIL-WITH-
        INSTRUCTION (exit 3): the improvement must be recorded explicitly. A
        measurement that did not run, ran nothing, or measured a different
        denominator is RED (exit 4), never PASS.
        --accept-gains keeps those regression checks but reports genuine gains
        as PASS without changing recorded floors. The gate uses this mode;
        improvements stay in its run output, not generated-evidence commits.
        A unit that lost a member is measured once more (same frozen binary,
        whole unit, same inputs). The loss stands unless that confirming run
        ALONE meets the unit's floor; then it is UNCONFIRMED: printed, counted
        in a durable ledger and in the gate's notices, and a member forgiven
        again within the window FAILS (see "Confirmation" below).
    parity_ratchet.py unconfirmed
        Print the UNCONFIRMED ledger: every forgiveness and escalation.
    parity_ratchet.py check --tier fast|gate
        fast: gnu-files:<the characterised files> and census-cold (seconds);
        gate: every unit the gate runs. Builds the binaries unless given.
    parity_ratchet.py record [--lower UNIT --because TEXT]...
        The ONLY writer of parity-ratchet.json. Measures every unit twice on a
        clean tree and refuses unless both runs agree, every census file was
        measured and romacs reported tests. Refuses to drop any recorded member
        unless that unit is named with --lower and a reason, which is kept in
        the file's `lowerings` (rule: lowering is only valid when the thing
        measured was replaced, and needs independent review).
    parity_ratchet.py verify [--root DIR] [--base REV]
        Static, no build (scripts/check-derived runs it): the file is internally
        consistent, and against the merge-base with master no recorded member
        was dropped without a `lowerings` entry naming it.
    parity_ratchet.py verify --candidate [--base REV]
        Static candidate check: no member, denominator or floor may decrease,
        even with a lowering record. Does not replay probe evidence or measure.

The checker never writes the ratchet file: a ratchet that updates itself is not
a ratchet.
"""
from __future__ import annotations
import os as _portable_os
from pathlib import Path as _PortablePath

import argparse
import collections
import concurrent.futures
import contextlib
import fcntl
import hashlib
import importlib.util
import io
import json
import math
import os
import pathlib
import platform
import pwd
import re
import shutil
import subprocess
import sys
import tempfile
import time

TOOLS = pathlib.Path(__file__).resolve().parent
DEFAULT_ROOT = TOOLS.parent
RATCHET_NAME = 'parity-ratchet.json'
GNU_PIN = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
GNU_SOURCE = pathlib.Path(os.environ.get('CENSUS_GNU_SOURCE', str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / '')))
CHARACTERISED = ('src/fns-tests.el', 'src/data-tests.el', 'lisp/subr-tests.el')
CENSUS_MODES = ('warm', 'cold')
GATE_SHARDS = 71
PREFIX = 'parity-ratchet:'
EXIT_PASS, EXIT_LOWER, EXIT_INVALID, EXIT_HIGHER, EXIT_NOT_MEASURED = 0, 1, 2, 3, 4
RECORD_COMMAND = 'python3 tools/parity_ratchet.py record'


class Refusal(Exception):
    """No verdict may be given: the measurement did not happen as claimed."""

    def __init__(self, status: int, message: str):
        super().__init__(message)
        self.status = status


def load_module(path: pathlib.Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def emacs() -> str:
    """The pinned GNU, never PATH. A bare 'emacs' fallback is how the distro
    build became the oracle and every frozen baseline got recorded against the
    wrong GNU; scripts/gnu_oracle.py is the one resolver and it fails closed."""
    explicit = os.environ.get('GNU_EMACS') or os.environ.get('ROMACS_FORCE_ORACLE_PATH')
    if explicit:
        return explicit
    oracle = load_module(pathlib.Path(__file__).resolve().parent.parent / 'scripts' / 'gnu_oracle.py',
                         'romacs_gnu_oracle')
    return str(oracle.require()['path'])


# --------------------------------------------------------------------------
# Measurement: the GNU suite

def suite_files(root: pathlib.Path) -> list[str]:
    return load_module(root / 'gnu-tests' / 'run.py', 'gnu_tests_run').suite_files()


def shard(files: list[str], spec: str) -> list[str]:
    k, n = (int(x) for x in spec.split('/'))
    if not 1 <= k <= n:
        raise Refusal(EXIT_INVALID, f'bad shard {spec!r}; want K/N with 1 <= K <= N')
    return sorted(files)[k - 1::n]


def measure_gnu(root: pathlib.Path, files: list[str], binary: str, jobs: int) -> dict:
    """Run run.py on FILES; return members (passing deftests) per file.

    run.py exits 1 whenever any test does not pass, so its status says only
    whether it measured (0/1) or refused (2/3/4). Refused is not measured.
    """
    if not files:
        raise Refusal(EXIT_NOT_MEASURED, 'no GNU test files selected; nothing would be measured')
    if not binary or not pathlib.Path(binary).is_file():
        raise Refusal(EXIT_NOT_MEASURED, f'romacs-gnu-ert binary {binary!r} does not exist')
    digest = sha256_file(binary)
    with tempfile.TemporaryDirectory(prefix='parity-gnu-') as tmp:
        out = pathlib.Path(tmp) / 'out'
        started = time.monotonic()
        for attempt in (1, 2):
            p = subprocess.run([sys.executable, str(root / 'gnu-tests' / 'run.py'), 'run',
                                '--harness', 'mini', '--bootstrap', 'ert', '--bin', binary,
                                '--jobs', str(jobs), '--out', str(out), *files],
                               cwd=root, capture_output=True, text=True)
            # Killed by a signal from outside (seen in the gate: exit -9 with no
            # OOM): no measurement happened, so measuring again is not a retry
            # of a verdict. Any other refusal is final.
            if p.returncode >= 0 or attempt == 2:
                break
            print(f'{PREFIX} note: run.py was killed by signal {-p.returncode}; measuring once more', file=sys.stderr)
            shutil.rmtree(out, ignore_errors=True)
        seconds = time.monotonic() - started
        if p.returncode not in (0, 1) or not (out / 'summary.json').is_file():
            raise Refusal(EXIT_NOT_MEASURED, f'gnu-tests/run.py did not measure (exit {p.returncode}):\n'
                          + (p.stderr or p.stdout)[-3000:])
        rows = [json.loads(line) for line in (out / 'results.jsonl').read_text().splitlines() if line.strip()]
    if sha256_file(binary) != digest:
        raise Refusal(EXIT_NOT_MEASURED, 'romacs-gnu-ert changed during measurement')
    seen = {r['file'] for r in rows}
    if seen != set(files):
        raise Refusal(EXIT_NOT_MEASURED, f'run.py reported {len(seen)} of {len(files)} requested files; '
                      f'missing {sorted(set(files) - seen)[:10]}')
    reported = sum(1 for r in rows if r.get('romacs') is not None)
    if reported == 0:
        raise Refusal(EXIT_NOT_MEASURED, f'romacs reported no test at all across {len(files)} files '
                      f'({len(rows)} GNU tests): zero tests executed is not a measurement')
    members = {f: sorted(r['test'] for r in rows if r['file'] == f and r['verdict'] == 'passed') for f in files}
    evidence = load_evidence(root)
    if any(value.get('method') == 'sealed-subject-v1' for value in evidence.values()):
        from vacuity_binding import verify_receipt
        _, manifest, _ = verify_receipt(root)
        if digest != manifest['binaries']['romacs']['file']['sha256']:
            raise Refusal(EXIT_INVALID, 'subject evidence binary differs from measured romacs binary; probe again')
    classes = {f"{r['file']}::{r['test']}": classify_member(evidence.get(f"{r['file']}::{r['test']}"), member_facts(r))
               for r in rows if r['verdict'] == 'passed'}
    verdicts = {(r['file'], r['test']): f"{r['verdict']}" + (f" ({r['missing']})" if r.get('missing') else '')
                + (f" -- {r['reason'][:200]}" if r.get('reason') else '') for r in rows}
    per_file_tests = collections.Counter(r['file'] for r in rows)
    return {'kind': 'gnu', 'harness': 'romacs-mini-ert', 'bootstrap_regime': 'gnu-ert',
            'files': sorted(files), 'gnu_tests': len(rows), 'romacs_reported': reported,
            'tests_per_file': dict(sorted(per_file_tests.items())), 'members': members,
            'classes': classes, 'facts_reported': all('file_loaded_clean' in r for r in rows),
            'verdicts': {f'{f}::{t}': v for (f, t), v in verdicts.items()}, 'binary_sha256': digest,
            'seconds': round(seconds, 1)}


def measure_real_ert(root: pathlib.Path, members: dict[str, list[str]], binary: str, jobs: int) -> dict:
    """How many reduced-harness passes also pass under GNU's real ert.el.

    Runs GNU ert.el with the explicit Romacs batch adapter and the pinned oracle
    as reference on every file holding a reduced-harness pass. A file whose
    real-ERT run aborts passes nothing; that is the measurement, not a
    refusal. No summary at all is a refusal. Adapted execution is labelled explicitly.
    """
    files = sorted(f for f, v in members.items() if v)
    of = sum(len(members[f]) for f in files)
    if not os.environ.get('ROMACS_GNU_ORACLE'):
        raise Refusal(EXIT_NOT_MEASURED, 'the real-ERT number needs the pinned GNU as $ROMACS_GNU_ORACLE '
                      '(scripts/gnu_oracle.py); it is never recorded as a guess')
    with tempfile.TemporaryDirectory(prefix='parity-real-ert-') as tmp:
        out = pathlib.Path(tmp) / 'out'
        p = subprocess.run([sys.executable, str(root / 'gnu-tests' / 'run.py'), 'run', '--harness', 'real',
                            '--bootstrap', 'ert',
                            '--ert-adapter', 'romacs-batch',
                            '--bin', binary, '--jobs', str(jobs), '--out', str(out), *files],
                           cwd=root, capture_output=True, text=True)
        if p.returncode not in (0, 1, 4) or not (out / 'summary.json').is_file():
            raise Refusal(EXIT_NOT_MEASURED, f'the real-ERT run did not measure (exit {p.returncode}):\n'
                          + (p.stderr or p.stdout)[-2000:])
        summary = json.loads((out / 'summary.json').read_text())
        rows = [json.loads(line) for line in (out / 'results.jsonl').read_text().splitlines() if line.strip()]
    adapter = summary.get('ert_adapter') or {}
    if (summary.get('is_gnu_ert') is not True or summary.get('engine') != 'romacs'
            or summary.get('bootstrap_regime') != 'gnu-ert'
            or summary.get('is_unmodified_gnu_ert') is not False
            or not isinstance(adapter, dict) or adapter.get('name') != 'romacs-batch'
            or adapter.get('automatic_window_isolation') is not False):
        raise Refusal(EXIT_NOT_MEASURED, 'the real-ERT run was not GNU ert.el on romacs')
    real = {(r['file'], r['test']) for r in rows if r.get('verdict') == 'passed'}
    passing = {f: sorted(n for n in members[f] if (f, n) in real) for f in files}
    passing = {f: v for f, v in passing.items() if v}
    return {'harness': BATCH_REAL_LABEL, 'is_gnu_ert': True, 'passed': sum(len(v) for v in passing.values()), 'of': of,
            'ert_adapter': adapter, 'is_unmodified_gnu_ert': False,
            'passing': passing, 'files': len(files), 'files_aborted': len(summary.get('infrastructure_errors') or {}),
            'oracle_sha256': (summary.get('gnu') or {}).get('sha256'), 'adapter_sha256': summary.get('adapter_sha256')}


def sha256_file(path) -> str:
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


# --------------------------------------------------------------------------
# Measurement: the Lisp census

def gnu_lisp_paths() -> set[str]:
    """GNU's lisp/*.el at the pin, read from GNU's object store (no copy)."""
    p = subprocess.run(['git', '-C', str(GNU_SOURCE), 'ls-tree', '-r', '-z', GNU_PIN, '--', 'lisp'],
                       capture_output=True)
    if p.returncode:
        raise Refusal(EXIT_NOT_MEASURED, f'cannot read GNU {GNU_PIN[:11]} from {GNU_SOURCE}: '
                      + p.stderr.decode(errors='replace')[-600:])
    paths = set()
    for entry in p.stdout.decode().split('\0'):
        if not entry:
            continue
        meta, path = entry.split('\t', 1)
        mode, kind, _ = meta.split()
        if kind == 'blob' and mode in ('100644', '100755') and path.endswith('.el'):
            paths.add(path)
    if not paths:
        raise Refusal(EXIT_NOT_MEASURED, 'GNU pin lists no lisp/*.el')
    return paths


# scripts/lisp-census.py kills a file after 10 s. A loading file takes under a
# second (max 0.73 s measured), but the gate runs beside other lanes' builds,
# so the ratchet allows 30 s: only a file that hangs pays for it, and a loaded
# file can no longer be lost to host load alone.
CENSUS_TIMEOUT = 30


def census_population(root: pathlib.Path) -> tuple[set[str], set[str]]:
    """Validate the pristine pin and declared support before choosing targets."""
    oracle = load_module(root / 'tools/lisp_oracle.py', 'census_lisp_oracle')
    diagnostic = io.StringIO()
    try:
        with contextlib.redirect_stdout(diagnostic), contextlib.redirect_stderr(diagnostic):
            status = oracle.check(root, GNU_SOURCE, GNU_PIN)
        if status:
            raise Refusal(EXIT_NOT_MEASURED, 'invalid census population:\n' + diagnostic.getvalue())
        pinned = oracle.pin_tree(GNU_SOURCE, GNU_PIN)
        support, problems = oracle.generated_support(root, GNU_SOURCE, GNU_PIN, pinned, required=True)
    except (SystemExit, ValueError, OSError) as error:
        raise Refusal(EXIT_NOT_MEASURED, f'cannot validate census population: {error}') from error
    if problems:
        raise Refusal(EXIT_NOT_MEASURED, 'invalid generated census support:\n  ' + '\n  '.join(problems))
    return {oracle.local_path(p) for p in pinned if p.endswith('.el')}, set(support)


def measure_census(root: pathlib.Path, mode: str, binary: str, workers: int, timeout: float = CENSUS_TIMEOUT) -> dict:
    """The census of scripts/lisp-census.py, through its own per-file runner.

    Its main() needs our git repository (the gate's mirror has none) and always
    rebuilds; this drives the same run_file/bootstrap_chain on a given binary.
    """
    if not binary or not pathlib.Path(binary).is_file():
        raise Refusal(EXIT_NOT_MEASURED, f'romacs-vm-census binary {binary!r} does not exist')
    lc = load_module(root / 'scripts' / 'lisp-census.py', 'lisp_census')
    if pathlib.Path(lc.ROOT).resolve() != root.resolve():
        raise Refusal(EXIT_INVALID, f'lisp-census.py resolves ROOT {lc.ROOT}, not {root}')
    gnu, support = census_population(root)
    files = [root / rel for rel in sorted(gnu | support)]
    if not files:
        raise Refusal(EXIT_NOT_MEASURED, f'no Lisp files under {root / "gnu-lisp"}')
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix='parity-census-') as tmp:
        manifest = pathlib.Path(tmp) / 'manifest.json'
        manifest.write_text(json.dumps(list(map(str, files))))
        p = subprocess.run([emacs(), '-Q', '--batch', '--script', str(root / 'scripts/lisp-census-oracle.el'),
                            '--manifest', str(manifest)], capture_output=True, text=True, timeout=600)
        if p.returncode:
            raise Refusal(EXIT_NOT_MEASURED, f'GNU form-count oracle failed (exit {p.returncode}): {p.stderr[-800:]}')
        oracle = json.loads(p.stdout.splitlines()[-1])['files']
        if [entry.get('path') for entry in oracle] != list(map(str, files)):
            raise Refusal(EXIT_NOT_MEASURED, 'GNU oracle file identities differ from the requested census targets')
        # The binary is copied so a concurrent rebuild cannot swap it mid-run.
        copy = pathlib.Path(tmp) / 'romacs-vm-census'
        shutil.copy2(binary, copy)
        digest = hashlib.sha256(copy.read_bytes()).hexdigest()
        load_paths = lc.load_path(root)
        preload = lc.bootstrap_chain(root) if mode == 'warm' else ()
        with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
            rows = list(pool.map(lambda pair: lc.run_file(pair[0], pair[1], copy, load_paths, timeout, preload),
                                 zip(files, oracle)))
        if hashlib.sha256(copy.read_bytes()).hexdigest() != digest:
            raise Refusal(EXIT_NOT_MEASURED, 'census binary changed during measurement')
    if [r.get('file') for r in rows] != sorted(gnu | support):
        raise Refusal(EXIT_NOT_MEASURED, 'census row identities differ from the requested targets')
    for row, entry in zip(rows, oracle):
        if row['total_forms'] != entry['form_count']:
            raise Refusal(EXIT_NOT_MEASURED, f"GNU form count changed: {row['file']}")
        n = row['forms_evaluated']
        if n is not None and (n > row['total_forms'] or (row['loaded'] and n != row['total_forms'])):
            raise Refusal(EXIT_NOT_MEASURED, f"form count mismatch: {row['file']}")
        if n is not None and row.get('mode') != mode:
            raise Refusal(EXIT_NOT_MEASURED, f"row measured in mode {row.get('mode')!r}, expected {mode!r}: {row['file']}")
    chain = lc.chain_consensus(rows) if mode == 'warm' else None
    if chain:  # how far the loadup chain reached, without host paths
        chain = [{'file': os.path.relpath(c['file'], root), 'loaded': c.get('loaded'),
                  'forms_completed': c.get('forms_completed')} for c in chain]
    reported = sum(1 for r in rows if r['forms_evaluated'] is not None and r['file'] in gnu)
    if reported == 0:
        raise Refusal(EXIT_NOT_MEASURED, f'the census binary reported on none of {len(gnu)} pristine GNU files')
    # Floor members retain their GNU source identities across checkout moves.
    loaded = sorted('lisp' + r['file'][len('gnu-lisp'):]
                    for r in rows if r['loaded'] and r['file'] in gnu)

    def why(r):
        f = r.get('first_failure') or {}
        return f"{f.get('classification')}: {(f.get('error') or '')[:160]}"
    return {'kind': 'census', 'mode': mode, 'files_measured': len(gnu), 'files_reported': reported,
            'files_executed': len(rows),
            'generated_support': {
                'total': len(support),
                'reported': sum(r['forms_evaluated'] is not None for r in rows if r['file'] in support),
                'loaded': sorted(r['file'] for r in rows if r['loaded'] and r['file'] in support),
                'unreported': sorted(r['file'] for r in rows if r['forms_evaluated'] is None and r['file'] in support)},
            'unreported': sorted(r['file'] for r in rows if r['forms_evaluated'] is None and r['file'] in gnu),
            'gnu_target_total': len(gnu), 'shared_total': len(gnu),
            'members': {f'census-{mode}': loaded},
            'verdicts': {('lisp' + r['file'][len('gnu-lisp'):] if r['file'] in gnu else r['file']):
                         ('loaded' if r['loaded'] else why(r)) for r in rows},
            'chain': chain, 'binary_sha256': digest, 'seconds': round(time.monotonic() - started, 1)}


# --------------------------------------------------------------------------
# Honest classification of GNU-suite members (pure; unit-tested)
#
# A reduced-harness pass (romacs-mini-ert, NOT GNU ert.el) is not evidence of
# capability until something shows it can fail. Each passing member gets a
# CLASS, derived -- never hand-written -- from the probe evidence in
# gnu-tests/MEMBER-EVIDENCE.json and the facts this measurement observed:
#
#   GENUINE           probe: breaking the subject breaks the test in both engines
#                     (or, for core special forms, every caught condition checked
#                     equal to pinned GNU by hand)
#   UNPROBEABLE       no probe evidence (a new pass, or a verdict this measurement
#                     contradicts): protected like GENUINE, never counted
#   VACUOUS           passes because the subject is missing
#   WRONG-ERROR       subject runs, a missing helper raises another error
#   WRONG-ERROR-DATA  GNU's error type, different data
#   HARNESS-SELF      tests the reduced harness, not romacs
#
# The last four are the VACUOUS BUDGET: recorded apart from the floor, and it
# may only DECREASE. A budget member that stops passing is PROGRESS (an honest
# failure replaced a fiction); a protected member that stops passing FAILS, as
# it always did. A member is COUNTED only if it is GENUINE and its test file
# evaluated every top-level form without error (the reduced harness keeps
# going past a failed `require`; GNU's `load` would not).

BUDGET_CLASSES = ('VACUOUS', 'WRONG-ERROR', 'WRONG-ERROR-DATA', 'HARNESS-SELF')
MEMBER_CLASSES = ('GENUINE', 'UNPROBEABLE') + BUDGET_CLASSES
EVIDENCE_NAME = 'MEMBER-EVIDENCE.json'
REDUCED_LABEL = 'romacs-mini-ert: REDUCED in-house harness, NOT GNU ert.el'
REAL_LABEL = 'gnu-ert: GNU ert.el, unmodified, pinned GNU oracle'
BATCH_REAL_LABEL = 'gnu-ert: GNU ert.el with Romacs batch adapter, pinned GNU oracle'


def load_evidence(root: pathlib.Path) -> dict:
    sys.path.insert(0, str(root / 'gnu-tests/tools'))
    from member_evidence import load
    try:
        return load(root)
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        raise Refusal(EXIT_INVALID, f'subject evidence refused: {error}') from error


def member_facts(row: dict) -> dict:
    """What a measurement observed about one passing member."""
    return {'file_loaded': bool(row.get('file_loaded_clean')), 'failed_forms': list(row.get('failed_forms') or [])[:10],
            'vacuous_catch': list(row.get('vacuous_catch') or [])}


def classify_member(evidence: dict | None, facts: dict) -> dict:
    """The class of one PASSING member, from its probe EVIDENCE and observed FACTS.

    What this measurement saw outranks the evidence: a pass satisfied by
    absent machinery is VACUOUS whatever the probe once said, and a probe
    verdict that rested on a void catch this measurement no longer shows is
    stale (UNPROBEABLE until probed again). Never GENUINE without evidence.
    """
    vacuous = list(facts.get('vacuous_catch') or [])
    if vacuous:
        cls, method = 'VACUOUS', 'void-catch'
        why = 'this measurement: should-error satisfied by absent machinery ' + ', '.join(vacuous[:6])
    elif evidence is None:
        cls, method = 'UNPROBEABLE', None
        why = 'no probe evidence for this member: protected like a genuine pass, never counted as one until probed'
    elif evidence.get('class') not in MEMBER_CLASSES:
        raise Refusal(EXIT_INVALID, f'probe evidence names unknown class {evidence.get("class")!r}')
    elif evidence['class'] in BUDGET_CLASSES and evidence.get('method') == 'void-catch':
        cls, method = 'UNPROBEABLE', 'stale'
        why = (f"probe verdict {evidence['class']} rested on a void catch this measurement no longer shows "
               f"(was: {str(evidence.get('why', ''))[:160]}); probe it again")
    else:
        cls, method, why = evidence['class'], evidence.get('method'), str(evidence.get('why', ''))
    loaded = facts.get('file_loaded') is True
    return {'class': cls, 'counted': cls == 'GENUINE' and loaded, 'file_loaded': loaded,
            'failed_forms': list(facts.get('failed_forms') or [])[:10], 'vacuous_catch': vacuous,
            'method': method, 'why': why[:300]}


def honest_counts(classes: dict) -> dict:
    by = collections.Counter(c['class'] for c in classes.values())
    return {'genuine': by['GENUINE'], 'counted': sum(1 for c in classes.values() if c['counted']),
            'unprobeable': by['UNPROBEABLE'], 'vacuous_budget': sum(by[c] for c in BUDGET_CLASSES),
            'by_class': {c: by[c] for c in MEMBER_CLASSES}}


def regeneration_problems(data: dict, evidence: dict) -> list[str]:
    """Stored classes that do not follow from EVIDENCE and the stored facts.

    The classes are written only by `record`; this recomputes each one from
    the probe evidence and the facts recorded beside it, so a hand edit -- or
    evidence that moved without a re-record -- is refused, not believed.
    """
    g = data.get('gnu_suite') or {}
    problems = []
    for key, stored in sorted((g.get('classes') or {}).items()):
        try:
            derived = classify_member(evidence.get(key), {k: stored.get(k) for k in
                                                          ('file_loaded', 'failed_forms', 'vacuous_catch')})
        except Refusal as e:
            problems.append(f'{key}: {e}')
            continue
        # Explanatory prose belongs to the run, not the acceptance policy.
        # Compare every semantic field, including class, method and counting.
        diff = sorted(k for k in derived.keys() | stored.keys()
                      if k != 'why' and derived.get(k) != stored.get(k))
        if diff:
            problems.append(f'{key}: recorded class {stored.get("class")!r} does not follow from the evidence '
                            f'(derived {derived["class"]!r}; differs in {diff}); re-record, never hand-edit')
    return problems


def honest_suite(files: list[str], tests_per_file: dict, members: dict, classes: dict, real_ert: dict) -> dict:
    """The schema-2 gnu_suite section from one measurement (record's only builder)."""
    passing = {f: sorted(v) for f, v in sorted(members.items()) if v}
    budget = {}
    for f, names in passing.items():
        b = sorted(n for n in names if classes[f'{f}::{n}']['class'] in BUDGET_CLASSES)
        if b:
            budget[f] = b
    total = sum(len(v) for v in passing.values())
    return {'files': sorted(files), 'file_count': len(files), 'gnu_tests': sum(tests_per_file.values()),
            'tests_per_file': dict(sorted(tests_per_file.items())),
            'reduced_harness': REDUCED_LABEL, 'passed': total,
            'characterised': {f: len(passing.get(f, [])) for f in CHARACTERISED},
            'passing': passing, 'vacuous_budget': budget,
            'classes': dict(sorted(classes.items())), 'honest': honest_counts(classes),
            'real_ert': real_ert}


# --------------------------------------------------------------------------
# The ratchet file and the comparison (pure; unit-tested)

def ratchet_path(root: pathlib.Path) -> pathlib.Path:
    return root / RATCHET_NAME


def load_ratchet(root: pathlib.Path, *, evidence: bool = True) -> dict:
    path = ratchet_path(root)
    if not path.is_file():
        raise Refusal(EXIT_INVALID, f'{path} does not exist; record it: {RECORD_COMMAND}')
    data = json.loads(path.read_text())
    problems = consistency(data)
    if evidence and not problems and data.get('schema') == 2:
        problems = regeneration_problems(data, load_evidence(root))
    if problems:
        raise Refusal(EXIT_INVALID, f'{path} is inconsistent:\n  ' + '\n  '.join(problems))
    return data


def consistency(data: dict) -> list[str]:
    """Problems that make a ratchet file meaningless; empty when sound."""
    out = []
    if data.get('schema') not in (1, 2):
        return [f'schema {data.get("schema")!r}, expected 1 or 2']
    g = data.get('gnu_suite') or {}
    per = g.get('passing') or {}
    if not isinstance(g.get('files'), list) or not g['files']:
        out.append('gnu_suite.files must list every GNU test file measured')
    else:
        stray = sorted(set(per) - set(g['files']))
        if stray:
            out.append(f'gnu_suite.passing names files outside gnu_suite.files: {stray[:5]}')
        if g.get('file_count') != len(g['files']):
            out.append(f'gnu_suite.file_count {g.get("file_count")} != {len(g["files"])} files')
    for f, names in per.items():
        if names != sorted(set(names)):
            out.append(f'gnu_suite.passing[{f}] is not sorted and unique')
        if not names:
            out.append(f'gnu_suite.passing[{f}] is empty; omit files with no floor')
    total = sum(len(v) for v in per.values())
    if g.get('passed') != total:
        out.append(f'gnu_suite.passed {g.get("passed")} != {total} recorded passing deftests')
    if not total:
        out.append('gnu_suite floor is zero: a ratchet at zero protects nothing')
    tpf = g.get('tests_per_file') or {}
    if g.get('files') and sorted(tpf) != sorted(g['files']):
        out.append('gnu_suite.tests_per_file must name exactly gnu_suite.files')
    if g.get('gnu_tests') != sum(tpf.values()):
        out.append(f'gnu_suite.gnu_tests {g.get("gnu_tests")} != sum of tests_per_file {sum(tpf.values())}')
    for f, n in (g.get('characterised') or {}).items():
        if n != len(per.get(f, [])):
            out.append(f'gnu_suite.characterised[{f}] = {n} but {len(per.get(f, []))} passing deftests are recorded')
    if sorted(g.get('characterised') or {}) != sorted(CHARACTERISED):
        out.append(f'gnu_suite.characterised must name {list(CHARACTERISED)}')
    for mode in CENSUS_MODES:
        c = (data.get('census') or {}).get(mode) or {}
        loaded = c.get('loaded') or []
        if loaded != sorted(set(loaded)):
            out.append(f'census.{mode}.loaded is not sorted and unique')
        if c.get('gnu_parity_loaded') != len(loaded):
            out.append(f'census.{mode}.gnu_parity_loaded {c.get("gnu_parity_loaded")} != {len(loaded)} files listed')
        if not loaded:
            out.append(f'census.{mode} floor is zero: a ratchet at zero protects nothing')
        for key in ('gnu_target_total', 'shared_total', 'files_measured'):
            if not isinstance(c.get(key), int) or c[key] <= 0:
                out.append(f'census.{mode}.{key} must be a positive denominator')
    if data.get('schema') == 2:
        out += honest_consistency(g)
    for i, low in enumerate(data.get('lowerings') or []):
        if not (low.get('unit') and isinstance(low.get('members'), list) and len(str(low.get('because', '')).split()) >= 6):
            out.append(f'lowerings[{i}] needs unit, members and a reason of at least six words')
    return out


def honest_consistency(g: dict) -> list[str]:
    """Schema 2: the classes, the budget, the counts and the two labelled numbers agree."""
    out = []
    per, budget, classes = g.get('passing') or {}, g.get('vacuous_budget'), g.get('classes')
    if not isinstance(budget, dict) or not isinstance(classes, dict):
        return ['schema 2 needs gnu_suite.vacuous_budget and gnu_suite.classes']
    if g.get('reduced_harness') != REDUCED_LABEL:
        out.append(f'gnu_suite.reduced_harness must say {REDUCED_LABEL!r}: the reduced count is always labelled')
    keys = {f'{f}::{n}' for f, names in per.items() for n in names}
    if set(classes) != keys:
        out.append(f'gnu_suite.classes must classify exactly the passing members '
                   f'({len(set(classes) ^ keys)} differ, e.g. {sorted(set(classes) ^ keys)[:3]})')
    for f, names in budget.items():
        if not names or names != sorted(set(names)):
            out.append(f'gnu_suite.vacuous_budget[{f}] must be sorted, unique and non-empty')
        if not set(names) <= set(per.get(f, [])):
            out.append(f'gnu_suite.vacuous_budget[{f}] names members that are not passing')
    for key, c in classes.items():
        if c.get('class') not in MEMBER_CLASSES:
            out.append(f'{key}: unknown class {c.get("class")!r}')
            continue
        f, n = key.split('::', 1)
        if (c['class'] in BUDGET_CLASSES) != (n in budget.get(f, [])):
            out.append(f'{key}: class {c["class"]} but {"not " if n not in budget.get(f, []) else ""}in the budget')
        if c.get('counted') != (c['class'] == 'GENUINE' and c.get('file_loaded') is True):
            out.append(f'{key}: counted must be exactly GENUINE and its file loaded')
    if not out and g.get('honest') != honest_counts(classes):
        out.append(f'gnu_suite.honest {g.get("honest")} != the counts of its classes {honest_counts(classes)}')
    r = g.get('real_ert')
    if not isinstance(r, dict):
        out.append('schema 2 needs gnu_suite.real_ert: the real-ERT number is recorded apart, always')
    else:
        if r.get('harness') not in (REAL_LABEL, BATCH_REAL_LABEL) or r.get('is_gnu_ert') is not True:
            out.append('gnu_suite.real_ert must identify its GNU ERT regime with is_gnu_ert true')
        adapter = r.get('ert_adapter') or {}
        if r.get('harness') == BATCH_REAL_LABEL:
            if (r.get('is_unmodified_gnu_ert') is not False or not isinstance(adapter, dict)
                    or adapter.get('name') != 'romacs-batch'
                    or adapter.get('automatic_window_isolation') is not False):
                out.append('gnu_suite.real_ert batch label requires the explicit Romacs batch adapter')
        elif (not isinstance(adapter, dict) or r.get('is_unmodified_gnu_ert') is False
              or adapter.get('name', 'none') != 'none'):
            out.append('gnu_suite.real_ert adapted execution cannot be labelled unmodified')
        passed = r.get('passed')
        if not isinstance(passed, int) or passed < 0 or r.get('of') != g.get('passed') or passed > (r.get('of') or 0):
            out.append(f'gnu_suite.real_ert.passed {passed!r} must count members of the {g.get("passed")} '
                       f'reduced-harness passes (of {r.get("of")!r})')
        real_members = {f'{f}::{n}' for f, names in (r.get('passing') or {}).items() for n in names}
        if len(real_members) != (passed if isinstance(passed, int) else -1) or not real_members <= keys:
            out.append('gnu_suite.real_ert.passing must list exactly its passed members, all reduced-harness passes')
    return out


def floors(data: dict) -> dict[str, list[str]]:
    """unit -> recorded members.

    gnu:FILE      the protected floor: passing members outside the budget
    budget:FILE   the vacuous budget (schema 2): may only shrink
    counted:FILE  members counted as genuine (schema 2)
    census-MODE   files the census loads
    """
    g = data['gnu_suite']
    budget = g.get('vacuous_budget') or {}
    out = {}
    for f, names in g['passing'].items():
        protected = [n for n in names if n not in set(budget.get(f, []))]
        if protected:
            out[f'gnu:{f}'] = protected
    for f, names in budget.items():
        out[f'budget:{f}'] = list(names)
    counted = collections.defaultdict(list)
    for key, c in (g.get('classes') or {}).items():
        if c.get('counted'):
            f, n = key.split('::', 1)
            counted[f].append(n)
    for f, names in counted.items():
        out[f'counted:{f}'] = sorted(names)
    for mode in CENSUS_MODES:
        out[f'census-{mode}'] = data['census'][mode]['loaded']
    return out


def compare(floor: dict[str, list[str]], measured: dict[str, list[str]], verdicts: dict[str, str]) -> dict:
    """Compare measured members with floors, unit by unit (the units measured).

    Returns {'lower': [...], 'higher': [...], 'equal': [...]}, each entry a
    dict with unit, floor, measured, lost, gained.
    """
    result = {'lower': [], 'higher': [], 'equal': []}
    for unit in sorted(measured):
        have, want = set(measured[unit]), set(floor.get(unit, []))
        entry = {'unit': unit, 'floor': len(want), 'measured': len(have),
                 'lost': sorted(want - have), 'gained': sorted(have - want)}
        entry['lost_why'] = {m: verdicts.get(m.split(':', 1)[-1] if unit.startswith('census') else
                                             f"{unit.split(':', 1)[1]}::{m}", '?') for m in entry['lost']}
        key = 'lower' if entry['lost'] else 'higher' if entry['gained'] else 'equal'
        result[key].append(entry)
    return result


def measured_units(m: dict) -> dict[str, list[str]]:
    """Every unit a measurement speaks for. For GNU files: the passing members
    split by class into the protected floor, the budget and the counted."""
    if m['kind'] != 'gnu':
        return dict(m['members'])
    classes = m.get('classes')
    out = {}
    for f in sorted(set(m.get('files') or []) | set(m['members'])):
        names = m['members'].get(f, [])
        if classes is None:  # a measurement from before classes: everything protected, nothing counted
            out[f'gnu:{f}'] = list(names)
            continue
        c = {n: classes.get(f'{f}::{n}') or {} for n in names}
        out[f'gnu:{f}'] = sorted(n for n in names if c[n].get('class') not in BUDGET_CLASSES)
        out[f'budget:{f}'] = sorted(n for n in names if c[n].get('class') in BUDGET_CLASSES)
        out[f'counted:{f}'] = sorted(n for n in names if c[n].get('counted'))
    return out


def floor_unit(unit: str) -> bool:
    """A unit whose loss FAILS (the budget's loss is progress)."""
    return not unit.startswith('budget:')


def denominators(data: dict, m: dict) -> list[str]:
    """The measurement must have measured what the floors were recorded over."""
    problems = []
    if m['kind'] == 'gnu':
        g = data['gnu_suite']
        unknown = sorted(set(m['files']) - set(g['files']))
        if unknown:
            problems.append(f'GNU test files with no recorded denominator: {unknown[:5]}')
        for f in m['files']:
            want = g['tests_per_file'].get(f)
            if want is not None and m['tests_per_file'].get(f) != want:
                problems.append(f'{f}: measured {m["tests_per_file"].get(f)} GNU tests, recorded {want}')
    else:
        c = data['census'][m['mode']]
        for key in ('gnu_target_total', 'shared_total', 'files_measured'):
            if m[key] != c[key]:
                problems.append(f'census-{m["mode"]}: {key} measured {m[key]}, recorded {c[key]}')
        if m['files_reported'] != m['files_measured']:
            problems.append(f'census-{m["mode"]}: partial pristine GNU report '
                            f'{m["files_reported"]}/{m["files_measured"]}; unreported {m.get("unreported")}')
        support = m.get('generated_support')
        if support and support['reported'] != support['total']:
            problems.append(f'census-{m["mode"]}: partial generated-support report '
                            f'{support["reported"]}/{support["total"]}; unreported {support["unreported"]}')
    return problems


def member_why(m: dict, unit: str, member: str) -> str:
    """The class this measurement gave a still-passing member, for messages."""
    if ':' not in unit:
        return ''
    c = (m.get('classes') or {}).get(f"{unit.split(':', 1)[1]}::{member}")
    if not c:
        return ''
    where = '' if c.get('file_loaded') else f" (file failed top-level form(s) {c.get('failed_forms')[:5]})"
    return f"class {c['class']}{where}: {c.get('why', '')[:160]}"


def recorded_class(data: dict, unit: str, member: str) -> str:
    if ':' not in unit:
        return ''
    c = (data['gnu_suite'].get('classes') or {}).get(f"{unit.split(':', 1)[1]}::{member}")
    return f" [recorded class {c['class']}]" if c else ''


def report(unit_label: str, data: dict, m: dict, *, accept_gains: bool = False) -> int:
    """Print the verdict for one measurement; return its exit status."""
    problems = denominators(data, m)
    support = m.get('generated_support')
    if support is not None:
        print(f'{PREFIX} {unit_label}: executed {m["files_executed"]} local targets; '
              f'generated support loaded {len(support["loaded"])}/{support["total"]}, '
              f'reported {support["reported"]}; not GNU parity credit')
    if problems:
        print(f'{PREFIX} NOT-MEASURED {unit_label}: the denominator moved, so the floors no longer describe '
              'this measurement:\n  ' + '\n  '.join(problems)
              + f'\n  if the measured thing was really replaced, record it explicitly: {RECORD_COMMAND}')
        return EXIT_NOT_MEASURED
    fl = floors(data)
    units = measured_units(m)
    result = compare(fl, units, m['verdicts'])
    entries = result['lower'] + result['higher'] + result['equal']
    protected = [u for u in units if u.startswith('gnu:') or u.startswith('census-')]
    floor_total = sum(len(fl.get(u, [])) for u in protected)
    measured_total = sum(len(units[u]) for u in protected)
    reported = m['romacs_reported'] if m['kind'] == 'gnu' else m['files_reported']
    denom = m['gnu_tests'] if m['kind'] == 'gnu' else m['files_measured']
    if not (reported > 0 and denom > 0 and units):
        print(f'{PREFIX} NOT-MEASURED {unit_label}: nothing was measured (units {len(units)}, '
              f'denominator {denom}, reported {reported}); that is RED')
        return EXIT_NOT_MEASURED
    conf = m.get('confirmation') or {}
    lower = [e for e in entries if floor_unit(e['unit']) and e['lost']]
    higher = [e for e in entries if floor_unit(e['unit']) and not e['lost'] and e['gained']]
    progress = [e for e in entries if not floor_unit(e['unit']) and e['lost']]
    growth = [e for e in entries if not floor_unit(e['unit']) and e['gained']]
    for e in lower:
        print(f"{PREFIX} FAIL {e['unit']}: floor {e['floor']}, measured {e['measured']} "
              f"(delta {e['measured'] - e['floor']:+d}); LOST {len(e['lost'])}:")
        for member in e['lost']:
            still = member_why(m, e['unit'], member)
            now = f'still passes but is no longer counted -- {still}' if still else e['lost_why'][member]
            print(f"    - {member}: now {now}{recorded_class(data, e['unit'], member)}")
        if e['gained']:
            print(f"    (also gained {len(e['gained'])}: {', '.join(e['gained'][:10])} -- a gain does not pay for a loss)")
        again = (conf.get('reproduced') or {}).get(e['unit'])
        if again is not None:
            print(f"    confirmation run also missed the floor: lost {', '.join(again[:10])} "
                  '-- two runs that each fail are not a pass')
        if e['unit'] in (conf.get('escalated') or {}):
            print(f"    ESCALATED: {conf['escalated'][e['unit']]}")
        if conf.get('invalid'):
            print(f"    not forgiven: the confirmation is not comparable: {conf['invalid']}")
    for e in growth:
        print(f"{PREFIX} BUDGET-INCREASE {e['unit']}: the vacuous budget may only decrease; "
              f"{e['floor']} recorded, {len(set(e['gained']) | set(fl.get(e['unit'], [])))} would be; new:")
        for member in e['gained']:
            print(f"    - {member}: {member_why(m, e['unit'], member)}")
    unconfirmed = conf.get('unconfirmed') or {}
    for unit, line in sorted(unconfirmed.items()):
        print(f'{PREFIX} UNCONFIRMED {unit}: {line}')
    for e in progress:
        print(f"{PREFIX} PROGRESS {e['unit']}: vacuous budget {e['floor']} -> {e['floor'] - len(e['lost'])}; "
              f"no longer passing for the wrong reason: "
              + ', '.join(f"{x} (now {e['lost_why'][x]})" for x in e['lost'][:20]))
    for e in higher:
        print(f"{PREFIX} HIGHER {e['unit']}: floor {e['floor']}, measured {e['measured']} "
              f"(delta {e['measured'] - e['floor']:+d}); gained: {', '.join(e['gained'][:20])}")
    status = (EXIT_LOWER if lower or growth else EXIT_HIGHER if higher or progress else EXIT_PASS)
    if accept_gains and status == EXIT_HIGHER:
        status = EXIT_PASS
        print(f'{PREFIX} gains retained in run output; recorded floors are unchanged')
    word = {EXIT_PASS: 'PASS', EXIT_LOWER: 'FAIL', EXIT_HIGHER: 'FAIL-WITH-INSTRUCTION'}[status]
    honest = ''
    if m['kind'] == 'gnu' and m.get('bootstrap_regime'):
        print(f'{PREFIX} harness {m["harness"]} bootstrap {m["bootstrap_regime"]}')
    if m['kind'] == 'gnu' and m.get('classes') is not None:
        b_floor = sum(len(fl.get(u, [])) for u in units if u.startswith('budget:'))
        b_now = sum(len(units[u]) for u in units if u.startswith('budget:'))
        counted = sum(len(units[u]) for u in units if u.startswith('counted:'))
        real = data['gnu_suite'].get('real_ert') or {}
        honest = (f' vacuous-budget {b_now}/{b_floor} counted {counted}'
                  + (f" real-ERT {real.get('passed')}/{real.get('of')}" if real else ' real-ERT unrecorded'))
    print(f'{PREFIX} {word} {unit_label}: measured {measured_total} floor {floor_total} '
          f'units {len(units)} denominator {denom} reported {reported} seconds {m["seconds"]} '
          f'unconfirmed {sum(len(v) for v in (conf.get("forgiven") or {}).values())}{honest}')
    if status == EXIT_HIGHER:
        print(f'{PREFIX} the numbers went UP (a gain, or a vacuous pass that now fails honestly). Record it in '
              f'this commit, explicitly:\n    {RECORD_COMMAND}\n  then commit {RATCHET_NAME}. '
              'The checker never raises its own floors or lowers its own budget.')
    if status == EXIT_LOWER:
        print(f'{PREFIX} a parity floor fell or the vacuous budget grew. Fix the regression; floors are never '
              'lowered to land.')
    return status


# --------------------------------------------------------------------------
# Building the binaries (when the caller did not pass them)

def cargo_build(root: pathlib.Path, args: list[str]) -> pathlib.Path:
    target = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', root / 'target'))
    p = subprocess.run(['cargo', 'build', '--offline', '--release', *args], cwd=root, capture_output=True, text=True)
    if p.returncode:
        raise Refusal(EXIT_NOT_MEASURED, f'cargo build {" ".join(args)} failed (exit {p.returncode}):\n{p.stderr[-2000:]}')
    return target / 'release'


def binaries(root: pathlib.Path, args) -> tuple[str, str]:
    gnu_ert, census = args.gnu_ert, args.census_bin
    if gnu_ert is None:
        gnu_ert = str(cargo_build(root, ['-p', 'romacs-gnu-tests', '--bin', 'romacs-gnu-ert']) / 'romacs-gnu-ert')
    if census is None:
        census = str(cargo_build(root, ['-p', 'romacs-vm', '--bin', 'romacs-vm-census']) / 'romacs-vm-census')
    return gnu_ert, census


def units_for(root: pathlib.Path, tier: str | None, unit: str | None) -> list[str]:
    if unit:
        return [unit]
    if tier == 'fast':
        return ['gnu-files:' + ','.join(CHARACTERISED), 'census-cold']
    if tier == 'gate':
        return [f'gnu-shard:{k}/{GATE_SHARDS}' for k in range(1, GATE_SHARDS + 1)] + ['census-warm', 'census-cold']
    raise Refusal(EXIT_INVALID, 'name --unit or --tier')


def measure_unit(root: pathlib.Path, unit: str, gnu_ert: str | None, census: str | None, jobs: int) -> dict:
    if unit.startswith('gnu-shard:'):
        return measure_gnu(root, shard(suite_files(root), unit.split(':', 1)[1]), gnu_ert, jobs)
    if unit.startswith('gnu-files:'):
        files = [f for f in unit.split(':', 1)[1].split(',') if f]
        missing = sorted(set(files) - set(suite_files(root)))
        if missing:
            raise Refusal(EXIT_INVALID, f'not GNU suite files: {missing}')
        return measure_gnu(root, files, gnu_ert, jobs)
    if unit.startswith('census-') and unit[7:] in CENSUS_MODES:
        return measure_census(root, unit[7:], census, jobs)
    raise Refusal(EXIT_INVALID, f'unknown unit {unit!r}')


# --------------------------------------------------------------------------
# Confirmation: a loss is measured again, and every forgiveness is counted
#
# A confirmation keeps the OBSERVED status. It never synthesizes a passing
# measurement out of two runs: a loss is cleared only when the confirming run,
# on its own, meets the floor of the unit it confirms (a GNU test file, or a
# census mode measured whole), with the same frozen binary, the same recorded
# denominators and byte-identical inputs. Anything else -- the loss reproduced,
# a DIFFERENT member lost, the confirmation not comparable -- leaves the first
# run's loss standing (FAIL). Confirmation restores only the floor members the
# first run lost; it never adds a gain.
#
# A cleared loss is UNCONFIRMED, not "fine": whether the host disturbed the
# first run or the product raced cannot be told apart from here. Each one is
# printed on success, appended to a durable ledger shared by every lane and
# gate, written to the gate's notice file when the gate names one, and a member
# forgiven more than UNCONFIRMED_ALLOWED times within UNCONFIRMED_WINDOW_DAYS
# FAILS. A forgiveness that cannot be recorded is not granted.
#
# Provenance against `record` (which writes floors and so demands more):
#   - clean tree: not required. check never writes; it measures the tree it
#     is run in, which in the gate is the frozen mirror of the receipt's tree
#     (no .git there). What check needs is that BOTH runs measured the SAME
#     inputs: input_fingerprint() hashes their content before the first run
#     and after the confirmation.
#   - frozen binaries: cmd_check copies each binary once; every run, first and
#     confirming, uses that copy, and the confirming run's binary_sha256 must
#     equal the first run's.
#   - identical member sets: not required, by construction (the first run lost
#     something). Required instead: the confirming run alone meets the floor.
#   - full selection: a census confirmation re-measures the WHOLE mode; a GNU
#     confirmation re-measures each affected test file whole, and a GNU file
#     is the unit (run.py runs each file in its own process; floors are per
#     file). denominators() is applied to the confirming run as to the first.

UNCONFIRMED_WINDOW_DAYS = 14
UNCONFIRMED_ALLOWED = 1   # per member per window; the next forgiveness FAILS
LEDGER_NAME = 'parity-ratchet-unconfirmed.jsonl'
FINGERPRINTED = ('gnu-lisp', 'rlisp', 'gnu-tests/test', 'gnu-tests/harness', 'gnu-tests/tools', 'gnu-tests/baseline',
                 'gnu-tests/run.py', 'gnu-tests/PIN.json', 'gnu-tests/EXCLUSIONS.toml', 'scripts/lisp-census.py',
                 'scripts/lisp-census-oracle.el', 'tools/parity_ratchet.py', 'gnu-tests/member-evidence-policy.json',
                 'parity-reference.toml',
                 RATCHET_NAME)


def input_fingerprint(root: pathlib.Path) -> str:
    """Content of everything a measurement reads besides the binary."""
    h = hashlib.sha256()
    for rel in FINGERPRINTED:
        p = root / rel
        paths = (sorted(q for q in p.rglob('*') if q.is_file() and '__pycache__' not in q.parts) if p.is_dir()
                 else [p] if p.is_file() else [])
        h.update(f'{rel}\0{len(paths)}\n'.encode())
        for q in paths:
            h.update(f'{q.relative_to(root)}\0'.encode() + hashlib.sha256(q.read_bytes()).digest())
    if os.environ.get('ROMACS_MEMBER_EVIDENCE') or os.environ.get('ROMACS_VACUITY_RECEIPT'):
        sys.path.insert(0, str(root / 'gnu-tests/tools'))
        from vacuity_binding import evidence_paths
        receipt, members = evidence_paths()
        for name, path in (('ROMACS_MEMBER_EVIDENCE', members), ('ROMACS_VACUITY_RECEIPT', receipt)):
            h.update(name.encode() + b'\0' + hashlib.sha256(path.read_bytes()).digest())
    return h.hexdigest()


def ledger_path() -> pathlib.Path:
    """Durable and shared by every lane and gate. The gate gives each test a
    private HOME, so the account's real home is read from the password
    database, not from $HOME."""
    override = os.environ.get('PARITY_RATCHET_LEDGER')
    if override:
        return pathlib.Path(override)
    return pathlib.Path(pwd.getpwuid(os.getuid()).pw_dir) / '.local' / 'state' / 'romacs' / LEDGER_NAME


def read_ledger(f) -> list[dict]:
    f.seek(0)
    rows = [json.loads(line) for line in f.read().splitlines() if line.strip()]
    for r in rows:
        # json accepts NaN/Infinity, and a NaN time compares false with every
        # window: such a row would silently stop counting. Refuse it instead.
        if not (isinstance(r, dict) and type(r.get('t')) in (int, float) and math.isfinite(r['t'])
                and isinstance(r.get('unit'), str) and isinstance(r.get('member'), str)):
            raise ValueError(f'malformed ledger row {str(r)[:120]}: rows need a finite time, unit and member')
    return rows


def lost_members(data: dict, m: dict) -> dict[str, list[str]]:
    """unit -> floor members this measurement did not reproduce."""
    fl = floors(data)
    have = measured_units(m)
    units = [f'gnu:{f}' for f in m['files']] if m['kind'] == 'gnu' else list(have)
    return {u: sorted(set(fl.get(u, [])) - set(have.get(u, []))) for u in units
            if set(fl.get(u, [])) - set(have.get(u, []))}


def merge_confirmation(data: dict, m: dict, again: dict) -> dict:
    """Pure: which of M's losses the confirming run AGAIN clears.

    A unit is cleared only if AGAIN, by itself, has every floor member of that
    unit; otherwise its first-run loss stands and AGAIN's own misses are kept
    as evidence. Returns {'cleared': unit -> members to restore (only first-run
    losses, never gains), 'reproduced': unit -> floor members AGAIN missed}.
    """
    fl = floors(data)
    second = measured_units(again)
    cleared, reproduced = {}, {}
    for unit, members in lost_members(data, m).items():
        missed = sorted(set(fl.get(unit, [])) - set(second.get(unit, [])))
        if missed:
            reproduced[unit] = missed
        else:
            cleared[unit] = sorted(set(members) & set(second.get(unit, [])))
    return {'cleared': cleared, 'reproduced': reproduced}


def forgive(m: dict, units: dict[str, list[str]], classes: dict | None = None) -> dict:
    """M with exactly these floor members restored, classed as the confirming
    run (CLASSES) classed them."""
    m = dict(m, members=dict(m['members']))
    if m.get('classes') is not None:
        m['classes'] = dict(m['classes'])
    for unit, members in units.items():
        key = unit[4:] if m['kind'] == 'gnu' else unit
        m['members'][key] = sorted(set(m['members'].get(key, [])) | set(members))
        if m.get('classes') is not None:
            for member in members:
                if f'{key}::{member}' in (classes or {}):
                    m['classes'][f'{key}::{member}'] = classes[f'{key}::{member}']
    return m


def incomparable(data: dict, m: dict, again: dict, lost: dict, fingerprint: str, root: pathlib.Path) -> str | None:
    """Why AGAIN cannot confirm anything about M, or None."""
    if again.get('kind') != m['kind'] or again.get('mode') != m.get('mode'):
        return 'the confirming run measured a different kind of unit'
    if m['kind'] == 'gnu' and sorted(again.get('files', [])) != sorted(u[4:] for u in lost):
        return 'the confirming run measured different GNU test files than the losses name'
    problems = denominators(data, again)
    if problems:
        return 'the confirming run measured a different denominator: ' + '; '.join(problems[:3])
    if not m.get('binary_sha256') or again.get('binary_sha256') != m['binary_sha256']:
        return (f"the confirming run used another binary (sha256 {str(again.get('binary_sha256'))[:12]}, "
                f"first run {str(m.get('binary_sha256'))[:12]})")
    if input_fingerprint(root) != fingerprint:
        return 'the measured inputs changed between the first run and the confirmation'
    return None


def account(cleared: dict[str, list[str]], m: dict, root: pathlib.Path) -> tuple[dict, dict, pathlib.Path]:
    """Record each forgiveness durably; split CLEARED into (allowed, escalated).

    Raises OSError/ValueError when the ledger cannot be read or written: the
    caller then forgives nothing.
    """
    path = ledger_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    now = time.time()
    try:
        tree = git(root, 'rev-parse', 'HEAD^{tree}').strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        tree = None  # the gate's mirror has no .git; root names it
    allowed, escalated, rows = {}, {}, []
    with open(path, 'a+') as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        history = read_ledger(f)
        for unit, members in sorted(cleared.items()):
            for member in members:
                prior = sum(1 for r in history if r.get('unit') == unit and r.get('member') == member
                            and r['t'] >= now - UNCONFIRMED_WINDOW_DAYS * 86400)
                outcome = 'escalated' if prior >= UNCONFIRMED_ALLOWED else 'forgiven'
                (escalated if outcome == 'escalated' else allowed).setdefault(unit, []).append((member, prior))
                key = f'{unit[4:]}::{member}' if m['kind'] == 'gnu' else member
                rows.append({'t': round(now, 3), 'at': time.strftime('%Y-%m-%dT%H:%M:%S%z'), 'unit': unit,
                             'member': member, 'outcome': outcome, 'prior_in_window': prior,
                             'first_run_verdict': m['verdicts'].get(key, '?'), 'tree': tree, 'root': str(root),
                             'binary_sha256': m.get('binary_sha256'), 'host': platform.node(),
                             'loadavg': loadavg()})
        f.seek(0, os.SEEK_END)
        f.write(''.join(json.dumps(r, sort_keys=True) + '\n' for r in rows))
        f.flush()
        os.fsync(f.fileno())
    return allowed, escalated, path


def gate_notice(lines: list[str]) -> None:
    """The gate prints this file on GREEN as well as RED (scripts/gate)."""
    target = os.environ.get('ROMACS_GATE_NOTICES')
    if target and lines:
        with open(target, 'a') as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            f.write(''.join(line + '\n' for line in lines))


def gate_evidence(lines: list[str]) -> None:
    """Every verdict, PASS included, into the gate's run directory (scripts/gate
    names the file). A passing nextest test's output is not retained in the
    run's events, so without this a PASS leaves no record of what it measured;
    the adapters refuse to pass unless their verdict line is in this file."""
    target = os.environ.get('ROMACS_GATE_EVIDENCE')
    if target:
        with open(target, 'a') as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            f.write(''.join(line + '\n' for line in lines))


def confirm_losses(root: pathlib.Path, data: dict, m: dict, gnu_ert, census, jobs: int, fingerprint: str) -> dict:
    """Measure the units behind any lost floor member once more; see above."""
    lost = lost_members(data, m)
    if not lost:
        return m
    conf = {'forgiven': {}, 'unconfirmed': {}, 'reproduced': {}, 'escalated': {}, 'invalid': None}
    m = dict(m, confirmation=conf)
    try:
        if m['kind'] == 'gnu':
            again = measure_gnu(root, sorted(u[4:] for u in lost), gnu_ert, jobs)
        else:
            again = measure_census(root, m['mode'], census, jobs)
    except Refusal as e:
        conf['invalid'] = f'the confirming run did not measure: {str(e)[:400]}'
        return m
    conf['invalid'] = incomparable(data, m, again, lost, fingerprint, root)
    if conf['invalid']:
        return m
    decision = merge_confirmation(data, m, again)
    conf['reproduced'] = decision['reproduced']
    if not decision['cleared']:
        return m
    try:
        allowed, escalated, path = account(decision['cleared'], m, root)
        window = f'{UNCONFIRMED_WINDOW_DAYS} days'
        for unit, pairs in allowed.items():
            conf['unconfirmed'][unit] = (
                f'{len(pairs)} floor member(s) lost in the first run; the confirming run met this unit\'s floor: '
                + ', '.join(f'{member} (forgiveness {prior + 1} of {UNCONFIRMED_ALLOWED} allowed in {window})'
                            for member, prior in pairs[:10])
                + f'. Cause NOT established (host disturbance and a product race look alike here); '
                  f'counted in {path}')
        for unit, pairs in escalated.items():
            conf['escalated'][unit] = (
                ', '.join(f'{member} was already forgiven {prior} time(s) in the last {window}' for member, prior in pairs[:10])
                + f' ({path}); a loss that keeps recurring is not forgiven again')
        gate_notice([f'{PREFIX} UNCONFIRMED {u}: {t}' for u, t in sorted(conf['unconfirmed'].items())]
                    + [f'{PREFIX} ESCALATED {u}: {t}' for u, t in sorted(conf['escalated'].items())])
    except (OSError, ValueError) as e:
        conf['unconfirmed'], conf['escalated'] = {}, {}
        conf['invalid'] = f'the forgiveness could not be recorded ({e}); an uncounted loss is never forgiven'
        return m
    conf['forgiven'] = {u: [member for member, _ in pairs] for u, pairs in allowed.items()}
    return forgive(m, conf['forgiven'], again.get('classes'))


def freeze(binary: str | None, into: pathlib.Path, name: str) -> str | None:
    """A private copy used for every run of this check, so no rebuild can mix runs."""
    if not binary or not pathlib.Path(binary).is_file():
        return binary  # measure_* refuses a missing binary itself
    return str(shutil.copy2(binary, into / name))


def cmd_check(args) -> int:
    root = pathlib.Path(args.root).resolve()
    data = load_ratchet(root)
    units = units_for(root, args.tier, args.unit)
    gnu_ert, census = args.gnu_ert, args.census_bin
    if gnu_ert is None and any(u.startswith('gnu-') for u in units):
        gnu_ert = str(cargo_build(root, ['-p', 'romacs-gnu-tests', '--bin', 'romacs-gnu-ert']) / 'romacs-gnu-ert')
    if census is None and any(u.startswith('census-') for u in units):
        census = str(cargo_build(root, ['-p', 'romacs-vm', '--bin', 'romacs-vm-census']) / 'romacs-vm-census')
    with tempfile.TemporaryDirectory(prefix='parity-check-bin-') as frozen:
        if any(u.startswith('gnu-') for u in units):
            gnu_ert = freeze(gnu_ert, pathlib.Path(frozen), 'romacs-gnu-ert')
        if any(u.startswith('census-') for u in units):
            census = freeze(census, pathlib.Path(frozen), 'romacs-vm-census')
        fingerprint = input_fingerprint(root)
        # Always measured here, never replayed from a file: a saved measurement
        # could say anything, so there is no option to supply one.
        measurements = [(u, measure_unit(root, u, gnu_ert, census, args.jobs)) for u in units]
        worst = EXIT_PASS
        for unit, m in measurements:
            m = confirm_losses(root, data, m, gnu_ert, census, args.jobs, fingerprint)
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                status = report(unit, data, m, accept_gains=getattr(args, 'accept_gains', False))
            print(buf.getvalue(), end='')
            try:
                gate_evidence(buf.getvalue().splitlines())
            except OSError as e:
                print(f'{PREFIX} NOT-MEASURED {unit}: the verdict could not be kept as gate evidence ({e}); '
                      'a verdict nobody can audit is not evidence')
                status = EXIT_NOT_MEASURED
            worst = max(worst, status, key=lambda s: {EXIT_PASS: 0, EXIT_HIGHER: 1, EXIT_LOWER: 2, EXIT_NOT_MEASURED: 3}[s])
    return worst


# --------------------------------------------------------------------------
# Recording (the only writer)

def git(root: pathlib.Path, *args: str) -> str:
    return subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True, text=True).stdout


def contamination(root: pathlib.Path, measured_only: bool = False) -> list[str]:
    """Anything that would make a measurement describe something other than HEAD."""
    measured = ['gnu-lisp', 'rlisp', 'gnu-tests', 'scripts', 'tools/parity_ratchet.py', 'parity-reference.toml']
    dirty = [line for line in git(root, 'status', '--porcelain', '--', *(measured if measured_only else [])).splitlines()
             if line[3:] != RATCHET_NAME]
    ignored = [line for line in git(root, 'status', '--porcelain', '--ignored', '--', 'gnu-lisp', 'rlisp', 'gnu-tests/test',
                                    'gnu-tests/baseline', 'gnu-tests/harness', 'gnu-tests/tools', 'scripts').splitlines()
               if line.startswith('!!') and not python_cache(line[3:])]
    return dirty + ignored


def python_cache(path: str) -> bool:
    """Only Python's own bytecode cache next to the scripts is harmless;
    anything ignored under gnu-lisp/ or the GNU import can change a measurement."""
    parts = pathlib.PurePosixPath(path.rstrip('/')).parts
    return (parts[:1] == ('scripts',) and '__pycache__' in parts
            and (parts[-1] == '__pycache__' or parts[-1].endswith('.pyc')))


def loadavg() -> list[float] | None:
    try:
        return [round(x, 2) for x in os.getloadavg()]
    except OSError:
        return None


def strip_volatile(m: dict) -> dict:
    return {k: v for k, v in m.items() if k not in ('seconds', 'binary_sha256')}


def apply_lowerings(old: dict, data: dict, lower: dict[str, str], verdicts: dict) -> list[dict]:
    """Carry OLD's lowering records into DATA and authorize every dropped member.

    Compares the UNION of old and new units: a unit absent from DATA (a GNU
    file whose last passing test failed) lost every member. Raises Refusal
    unless each losing unit is named in LOWER; returns the raised units.
    """
    data['lowerings'] = list(old.get('lowerings') or [])
    result = compare(floors(old), {**{u: [] for u in floors(old)}, **floors(data)}, verdicts)
    refused = []
    lowered = {e['unit']: set(e['lost']) for e in result['lower'] if e['unit'] in lower}
    refused += budget_growth(result, lowered)
    for e in result['lower']:
        unit = e['unit']
        if not floor_unit(unit):
            continue  # the vacuous budget shrinking is progress, never a lowering
        if unit not in lower:
            refused.append(f"{unit}: would drop {len(e['lost'])} ({', '.join(e['lost'][:8])})")
        else:
            data['lowerings'].append({'unit': unit, 'members': e['lost'], 'because': lower[unit],
                                      'from_commit': (old.get('provenance') or {}).get('commit'),
                                      'at_commit': (data.get('provenance') or {}).get('commit')})
    unused = sorted(set(lower) - {e['unit'] for e in result['lower'] if floor_unit(e['unit'])})
    if unused:
        refused.append(f'--lower names units that lost nothing: {unused}')
    if refused:
        needed = sorted({e['unit'] for e in result['lower'] if floor_unit(e['unit'])} - set(lower))
        if needed:
            refused.append('if, and only if, the owner authorises lowering these units, the command is:\n    '
                           + RECORD_COMMAND + ''.join(f' \\\n      --lower {u} --because "REASON"' for u in needed))
        raise Refusal(EXIT_LOWER, 'refusing to lower floors:\n  ' + '\n  '.join(refused)
                      + '\n  a regression is fixed, not recorded. Only if the measured thing was replaced '
                      '(e.g. lisp/ swapped to the pin) name the unit: --lower UNIT --because "why"')
    return [e for e in result['higher'] if floor_unit(e['unit'])]


def budget_growth(result: dict, lowered: dict[str, set]) -> list[str]:
    """Members added to the vacuous budget that no lowering moved there.

    The budget may only decrease. The one way a member enters it is the
    owner reclassifying a PROTECTED floor member through a lowering record
    for that file's gnu: unit, in the same step: the floor and the budget
    then change together and nothing is gained. A brand-new pass never
    enters the budget, and neither does a member nobody lowered.
    """
    out = []
    for e in result['lower'] + result['higher'] + result['equal']:
        if floor_unit(e['unit']) or not e['gained']:
            continue
        f = e['unit'].split(':', 1)[1]
        moved = lowered.get(f'gnu:{f}', set())
        new = [m for m in e['gained'] if m not in moved]
        if new:
            out.append(f"{e['unit']}: the vacuous budget may only decrease; {len(new)} member(s) would enter it "
                       f"without a lowering that moves them out of gnu:{f}: {', '.join(new[:8])}")
    return out


def preserve_identical_provenance(old, data):
    """An identical remeasurement is a no-op, not a new timestamped record.

    Preserve the original record's actual run metadata only when the complete
    payload and source/binary bindings agree. Fresh runs still execute fully
    and print their own results; this does not reuse measurement outcomes.
    """
    keys = ('commit', 'tree', 'gnu_ert_sha256', 'census_sha256', 'repeat_runs_identical')
    before, after = (old or {}).get('provenance', {}), data.get('provenance', {})
    if (old is not None
            and {k: v for k, v in old.items() if k != 'provenance'} ==
                {k: v for k, v in data.items() if k != 'provenance'}
            and all(before.get(key) is not None and before.get(key) == after.get(key) for key in keys)):
        data['provenance'] = dict(before)


def cmd_record(args) -> int:
    root = pathlib.Path(args.root).resolve()
    dirty = contamination(root)
    if dirty:
        raise Refusal(EXIT_INVALID, 'refusing to record floors from a tree that is not exactly HEAD '
                      '(commit or remove these first):\n  ' + '\n  '.join(dirty[:20]))
    lower = dict(zip(args.lower or [], args.because or []))
    if len(args.lower or []) != len(args.because or []):
        raise Refusal(EXIT_INVALID, 'every --lower UNIT needs its own --because TEXT')
    gnu_ert, census = binaries(root, args)
    # Freeze both binaries for the WHOLE recording: every run uses these copies
    # and the provenance hashes them, so a concurrent rebuild cannot mix runs.
    frozen = tempfile.TemporaryDirectory(prefix='parity-record-bin-')
    gnu_ert = str(shutil.copy2(gnu_ert, pathlib.Path(frozen.name) / 'romacs-gnu-ert'))
    census = str(shutil.copy2(census, pathlib.Path(frozen.name) / 'romacs-vm-census'))
    frozen_sha = {b: hashlib.sha256(pathlib.Path(b).read_bytes()).hexdigest() for b in (gnu_ert, census)}
    commit_before = git(root, 'rev-parse', 'HEAD').strip()
    files = suite_files(root)
    runs = []
    load_before = loadavg()
    started = time.monotonic()
    for attempt in range(args.repeat):
        gnu = measure_gnu(root, files, gnu_ert, args.jobs)
        cen = {mode: measure_census(root, mode, census, args.jobs) for mode in CENSUS_MODES}
        for mode, c in cen.items():
            if c['unreported']:
                raise Refusal(EXIT_NOT_MEASURED, f'census-{mode}: {len(c["unreported"])} GNU files produced no report '
                              f'(timeouts?) e.g. {c["unreported"][:5]}; a partial census is not a floor')
        runs.append((gnu, cen))
        print(f'{PREFIX} record run {attempt + 1}/{args.repeat}: GNU passed '
              f'{sum(len(v) for v in gnu["members"].values())}, warm {len(cen["warm"]["members"]["census-warm"])}, '
              f'cold {len(cen["cold"]["members"]["census-cold"])}', flush=True)
    first = runs[0]
    for gnu, cen in runs[1:]:
        if (gnu['members'] != first[0]['members'] or gnu['classes'] != first[0]['classes']
                or any(cen[m]['members'] != first[1][m]['members'] for m in CENSUS_MODES)):
            raise Refusal(EXIT_NOT_MEASURED, 'two measurements of the same tree disagree; a flaky number is not a floor')
    gnu, cen = first
    if not gnu.get('facts_reported'):
        raise Refusal(EXIT_NOT_MEASURED, 'run.py reported no file-load facts; no member can be counted without them')
    real_ert = measure_real_ert(root, gnu['members'], gnu_ert, args.jobs)
    print(f"{PREFIX} real-ERT ({real_ert['harness']}): {real_ert['passed']}/{real_ert['of']} reduced-harness passes "
          f"also pass under GNU ert.el ({real_ert['files_aborted']} of {real_ert['files']} files aborted)", flush=True)
    if any(hashlib.sha256(pathlib.Path(b).read_bytes()).hexdigest() != h for b, h in frozen_sha.items()):
        raise Refusal(EXIT_NOT_MEASURED, 'a frozen binary changed during recording')
    if any(c['binary_sha256'] != frozen_sha[census] for _, cs in runs for c in cs.values()):
        raise Refusal(EXIT_NOT_MEASURED, 'a census run used a different binary than the frozen one')
    head = git(root, 'rev-parse', 'HEAD').strip()
    after = contamination(root, measured_only=True)
    if after or head != commit_before:
        raise Refusal(EXIT_NOT_MEASURED, 'the measured inputs changed while recording (HEAD moved or files changed); '
                      'nothing was written:\n  ' + '\n  '.join(after[:20]))
    data = {
        'schema': 2,
        'about': 'Parity floors. Written ONLY by `python3 tools/parity_ratchet.py record`; checked in the gate '
                 '(gnu-tests/runner/tests/parity_ratchet.rs, crates/romacs-vm/tests/parity_ratchet.rs) and by '
                 'scripts/check-derived. gnu_suite.passed counts REDUCED-harness passes (romacs-mini-ert, NOT GNU '
                 'ert.el); gnu_suite.real_ert counts passes under GNU ert.el; gnu_suite.honest.counted counts GENUINE '
                 'members whose file loaded. A lost protected member fails; a gain fails until recorded; the '
                 'vacuous_budget may only decrease, and a budget member that stops passing is progress.',
        'gnu_suite': honest_suite(gnu['files'], gnu['tests_per_file'], gnu['members'], gnu['classes'], real_ert),
        'census': {mode: {'gnu_parity_loaded': len(c['members'][f'census-{mode}']),
                          'gnu_target_total': c['gnu_target_total'], 'shared_total': c['shared_total'],
                          'files_measured': c['files_measured'], 'chain': c['chain'],
                          'loaded': c['members'][f'census-{mode}']}
                   for mode, c in cen.items()},
        'lowerings': [],
        'provenance': {
            'commit': git(root, 'rev-parse', 'HEAD').strip(), 'tree': git(root, 'rev-parse', 'HEAD^{tree}').strip(),
            'gnu_ert_sha256': frozen_sha[gnu_ert], 'census_sha256': frozen_sha[census],
            'repeat_runs_identical': args.repeat, 'host': platform.node(),
            'loadavg_before': load_before, 'loadavg_after': loadavg(),
            'seconds_total': round(time.monotonic() - started, 1),
            'seconds_per_run': [{'gnu_suite': g['seconds'], 'census_warm': c['warm']['seconds'],
                                 'census_cold': c['cold']['seconds']} for g, c in runs],
            'recorded_at': time.strftime('%Y-%m-%dT%H:%M:%S%z')},
    }
    path = ratchet_path(root)
    old = json.loads(path.read_text()) if path.is_file() else None
    if args.out:  # a reviewable proposal: every check below still applies; the ratchet is not touched
        path = pathlib.Path(args.out).resolve()
    if old is not None:
        verdicts = {**gnu['verdicts'], **{k: v for c in cen.values() for k, v in c['verdicts'].items()}}
        for e in apply_lowerings(old, data, lower, verdicts):
            print(f"{PREFIX} raising {e['unit']}: {e['floor']} -> {e['measured']}")
    elif lower:
        raise Refusal(EXIT_INVALID, '--lower given but no floors are recorded yet')
    problems = consistency(data)
    if problems:
        raise Refusal(EXIT_INVALID, 'the recorded floors would be inconsistent:\n  ' + '\n  '.join(problems))
    preserve_identical_provenance(old, data)
    tmp = path.with_suffix('.json.tmp')
    tmp.write_text(json.dumps(data, indent=1, sort_keys=True) + '\n')
    tmp.replace(path)
    g = data['gnu_suite']
    print(f"{PREFIX} recorded {path.name}: real-ERT {g['real_ert']['passed']}/{g['real_ert']['of']} ({REAL_LABEL}); "
          f"reduced-harness passes {g['passed']} ({REDUCED_LABEL}): GENUINE {g['honest']['genuine']}, counted "
          f"{g['honest']['counted']} (file loaded), UNPROBEABLE {g['honest']['unprobeable']}, vacuous budget "
          f"{g['honest']['vacuous_budget']}", flush=True)
    print(f"{PREFIX} recorded {path.name}: GNU suite passed {g['passed']}/{g['gnu_tests']} in {g['file_count']} files "
          f"(characterised {g['characterised']}); census warm {data['census']['warm']['gnu_parity_loaded']}/"
          f"{data['census']['warm']['gnu_target_total']}, cold {data['census']['cold']['gnu_parity_loaded']}/"
          f"{data['census']['cold']['gnu_target_total']}. Review the diff and commit it.")
    return 0


# --------------------------------------------------------------------------
# Static verification (scripts/check-derived)

# The gate's ratchet tests, pinned by content. Source-level checks for
# `#[test]`, `cfg`, `ignore` and friends were defeated twice in review (a
# whitespace spelling `#! [cfg(any())]` compiled every test out while the
# text still matched), so any edit to these files must also update the pin
# here, in the same reviewed commit. That is the point: the gate cannot stop
# running the ratchet without someone saying so.
GATE_TESTS = {
    'gnu-tests/runner/tests/candidate_ratchet.rs':
        '446697cc777e43bbaf3899b9dddb6e350a54776c8edae25c696daf0cdd7caba6',
    'gnu-tests/runner/tests/parity_ratchet.rs':
        'b69c150adc1f71bed8385df15d0b48e87a2abdef96b920f3c6c4a468827c76ab',
    'crates/romacs-vm/tests/parity_ratchet.rs':
        '43ced4188fdb6b2929503d0b29b0c519bdc6e57ae470327e1e12240822144366',
}


def wiring(root: pathlib.Path) -> list[str]:
    """The gate must still run every unit: a ratchet that is not executed is decoration.

    Each gate test file must be byte-identical to its pinned content. That the
    gate's inventory really contains the tests is shown by its receipt
    (`passed`), not here.
    """
    problems = []
    for rel, pinned in GATE_TESTS.items():
        path = root / rel
        if not path.is_file():
            problems.append(f'{rel} is missing; the gate would not run the ratchet')
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != pinned:
            problems.append(f'{rel} was edited (sha256 {actual[:12]}, pinned {pinned[:12]}); if the gate must run '
                            'the ratchet differently, update GATE_TESTS in tools/parity_ratchet.py in the same commit')
    return problems


def lowering_problems(old: dict, new: dict, label: str, merge: bool = False) -> list[str]:
    """Problems with the step OLD -> NEW of the ratchet file.

    Every member lost in the step must be named by a lowering record ADDED in
    that step; records already present cannot authorize a fresh loss (a
    member lowered once, restored, then lost again needs a new record), and
    existing records must be kept verbatim. In an ordinary step the old
    records are a prefix of the new. In a MERGE step (OLD is one parent) each
    of OLD's records must survive verbatim, but the other side's records may
    sit before them: two branches that each recorded a lowering cannot both
    be a prefix of the merged list. Records the merge holds beyond OLD's are
    the ones added in this step, relative to OLD.
    """
    before, after = old.get('lowerings') or [], new.get('lowerings') or []
    problems = []
    if merge:
        added = list(after)
        for low in before:
            if low in added:
                added.remove(low)
            else:
                problems.append(f'{label}: earlier lowering records were changed or deleted')
                break
    else:
        if after[:len(before)] != before:
            problems.append(f'{label}: earlier lowering records were changed or deleted')
        added = after[len(before):]
    covered = collections.defaultdict(set)
    for low in added:
        covered[low['unit']].update(low['members'])
    result = compare(floors(old), {**{u: [] for u in floors(old)}, **floors(new)}, {})
    problems += [f'{label}: {p}' for p in budget_growth(result, covered)]
    for e in result['lower']:
        if not floor_unit(e['unit']):
            continue
        hidden = [m for m in e['lost'] if m not in covered[e['unit']]]
        if hidden:
            problems.append(f"{label}: {e['unit']}: {len(hidden)} dropped ({', '.join(hidden[:8])})")
    return problems


def ratchet_at(root: pathlib.Path, rev: str) -> dict | None:
    """The ratchet file at REV, or None when REV has no such file."""
    p = subprocess.run(['git', '-C', str(root), 'rev-parse', '--verify', '--quiet', f'{rev}:{RATCHET_NAME}'],
                       capture_output=True, text=True)
    if p.returncode:
        return None
    return json.loads(git(root, 'cat-file', 'blob', p.stdout.strip()))


def candidate_problems(old: dict, new: dict, label: str) -> list[str]:
    """Static monotonicity only: evidence is measured after landing, not replayed."""
    problems = [f'{label}: {p}' for p in consistency(new)]
    if problems:
        return problems
    if old.get('schema') != new.get('schema'):
        problems.append(f'{label}: candidate changes the ratchet schema')
    if old.get('lowerings', []) != new.get('lowerings', []):
        problems.append(f'{label}: candidate changes lowering records; no floor lowering is permitted')
    before, after = floors(old), floors(new)
    for side, data in ((before, old), (after, new)):
        g = data['gnu_suite']
        side.update({f'all:{f}': names for f, names in g['passing'].items()})
        side.update({f'real:{f}': names for f, names in (g.get('real_ert') or {}).get('passing', {}).items()})
        side['gnu-files'] = g['files']
    for unit, names in sorted(before.items()):
        if unit.startswith('budget:'):
            continue
        lost = sorted(set(names) - set(after.get(unit, [])))
        if lost:
            problems.append(f'{label}: {unit}: {len(lost)} dropped ({", ".join(lost[:8])})')
    result = compare(floors(old), {**{u: [] for u in floors(old)}, **floors(new)}, {})
    problems += [f'{label}: {p}' for p in budget_growth(result, {})]
    for f, count in old['gnu_suite']['tests_per_file'].items():
        if new['gnu_suite']['tests_per_file'].get(f, 0) < count:
            problems.append(f'{label}: {f}: GNU test denominator lowered')
    for mode in CENSUS_MODES:
        for key in ('gnu_target_total', 'shared_total', 'files_measured'):
            if new['census'][mode][key] < old['census'][mode][key]:
                problems.append(f'{label}: census-{mode}: {key} lowered')
    return problems


def cmd_verify(args) -> int:
    root = pathlib.Path(args.root).resolve()
    candidate = getattr(args, 'candidate', False)
    data = load_ratchet(root, evidence=not candidate)
    unwired = wiring(root)
    if unwired:
        print(f'{PREFIX} verify FAIL: the gate no longer runs the ratchet:\n  ' + '\n  '.join(unwired), file=sys.stderr)
        return EXIT_LOWER
    base = args.base
    try:
        if base is None:
            base = git(root, 'merge-base', 'HEAD', 'master').strip()
        base = git(root, 'rev-parse', '--verify', f'{base}^{{commit}}').strip()
        # EVERY commit since the base, side branches included, oldest first;
        # each line is `commit parent...`.
        graph = [line.split() for line in
                 git(root, 'rev-list', '--parents', '--topo-order', '--reverse', f'{base}..HEAD').splitlines()]
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        raise Refusal(EXIT_INVALID, f'cannot resolve the base to verify against ({args.base or "merge-base with master"}): '
                      f'{getattr(e, "stderr", "") or e}')
    # Every version since the base -- on the first-parent chain AND on any
    # side branch merged into it -- is compared with EVERY parent that had the
    # file (both sides of a merge), then the working tree with HEAD: a floor
    # raised anywhere on the branch is protected as much as one inherited from
    # master, and a merge cannot drop either side's floors. A commit whose
    # parents never had the file is not a deletion: a lane that branched
    # before the ratchet existed and merged master forward walks its own
    # pre-ratchet commits here.
    cache = {}

    def at(rev):
        if rev not in cache:
            cache[rev] = ratchet_at(root, rev)
        return cache[rev]
    steps = [(c, c[:11], ps) for c, *ps in graph]
    problems, compared = [], 0
    for rev, label, ps in steps + [(None, 'working tree', ['HEAD'])]:
        version = data if rev is None else at(rev)
        for parent in ps:
            old = at(parent)
            if old is None:
                continue
            if version is None:
                problems.append(f'{label}: {RATCHET_NAME} was deleted (present in parent {parent[:11]})')
            elif version != old:
                step = f'{parent[:11]} -> {label}'
                if candidate:
                    problems += candidate_problems(old, version, step)
                else:
                    problems += lowering_problems(old, version, step, merge=len(ps) > 1)
                compared += 1
    if problems:
        instruction = ('candidate verification permits no member loss or floor lowering' if candidate else
                       f'floors are lowered only by `{RECORD_COMMAND} --lower UNIT --because TEXT`')
        heading = 'candidate violates monotonicity' if candidate else 'floors lowered without a lowering record'
        print(f'{PREFIX} verify FAIL: {heading}:\n  ' + '\n  '.join(problems) + '\n  ' + instruction, file=sys.stderr)
        return EXIT_LOWER
    kind = 'candidate (static, not a measurement)' if candidate else 'verify'
    print(f'{PREFIX} {kind}: consistent; {compared} recorded change(s) since {base[:11]}, none lowers a floor '
          f'silently (GNU suite {data["gnu_suite"]["passed"]}, warm {data["census"]["warm"]["gnu_parity_loaded"]}, '
          f'cold {data["census"]["cold"]["gnu_parity_loaded"]})')
    return 0


def cmd_unconfirmed() -> int:
    path = ledger_path()
    if not path.exists():
        print(f'{PREFIX} no loss has been forgiven ({path} does not exist)')
        return 0
    with open(path) as f:
        rows = read_ledger(f)
    now = time.time()
    per = collections.Counter((r.get('unit'), r.get('member')) for r in rows
                              if r['t'] >= now - UNCONFIRMED_WINDOW_DAYS * 86400)
    for r in rows:
        print(f"{r.get('at')} {str(r.get('outcome')).upper():9} {r.get('unit')} {r.get('member')} "
              f"first run: {r.get('first_run_verdict')} tree {str(r.get('tree'))[:11]} load {r.get('loadavg')}")
    print(f'{PREFIX} {len(rows)} recorded in {path}; within {UNCONFIRMED_WINDOW_DAYS} days: '
          + (', '.join(f'{u} {m} x{n}' for (u, m), n in per.most_common(20)) or 'none'))
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest='cmd', required=True)
    for name in ('check', 'record', 'verify'):
        p = sub.add_parser(name)
        p.add_argument('--root', default=str(DEFAULT_ROOT))
        if name != 'verify':
            p.add_argument('--gnu-ert', help='prebuilt romacs-gnu-ert (default: cargo build --release)')
            p.add_argument('--census-bin', help='prebuilt romacs-vm-census (default: cargo build --release)')
            p.add_argument('--jobs', type=int, default=8)
    c = sub.choices['check']
    c.add_argument('--unit')
    c.add_argument('--tier', choices=['fast', 'gate'])
    c.add_argument('--accept-gains', action='store_true',
                   help='allow improvements without rewriting floors; all losses and budget growth still fail')
    r = sub.choices['record']
    r.add_argument('--repeat', type=int, default=2)
    r.add_argument('--lower', action='append')
    r.add_argument('--because', action='append')
    r.add_argument('--out', help='write the recorded file here instead of parity-ratchet.json (a proposal for review)')
    sub.choices['verify'].add_argument('--base', help='compare against this revision (default: merge-base with master)')
    sub.choices['verify'].add_argument('--candidate', action='store_true',
                                      help='static no-member-loss/no-lowering check; no probe or evidence replay')
    sub.add_parser('unconfirmed', help='print the ledger of forgiven (UNCONFIRMED) and escalated losses')
    m = sub.add_parser('measure', help='measure one unit and print the measurement JSON (tests and debugging)')
    m.add_argument('--root', default=str(DEFAULT_ROOT))
    m.add_argument('--unit', required=True)
    m.add_argument('--gnu-ert')
    m.add_argument('--census-bin')
    m.add_argument('--jobs', type=int, default=8)
    args = ap.parse_args(argv)
    try:
        if args.cmd == 'check':
            return cmd_check(args)
        if args.cmd == 'record':
            if args.repeat < 2:
                raise Refusal(EXIT_INVALID, '--repeat must be at least 2: one run cannot show a number is stable')
            return cmd_record(args)
        if args.cmd == 'verify':
            return cmd_verify(args)
        if args.cmd == 'unconfirmed':
            return cmd_unconfirmed()
        if args.cmd == 'measure':
            root = pathlib.Path(args.root).resolve()
            print(json.dumps(measure_unit(root, args.unit, args.gnu_ert, args.census_bin, args.jobs), indent=1))
            return 0
    except Refusal as e:
        print(f'{PREFIX} NOT-MEASURED/REFUSED (exit {e.status}): {e}', file=sys.stderr)
        print(f'{PREFIX} NOT-MEASURED: no verdict was reached, and that is RED')
        return e.status
    return EXIT_INVALID


if __name__ == '__main__':
    sys.exit(main())
