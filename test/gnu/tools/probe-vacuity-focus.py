#!/usr/bin/env python3
"""Collect limited mutation observations, never automatically reclassify a budget.

Uses unchanged source-extracted deftests and disposable in-memory mutants.
The whole-body control intentionally deletes assertions: survival is NOT a
vacuity classifier. See R239-VACUITY-EVIDENCE.md for the stopping boundary.
Run on a clean committed tree or a gate-owned frozen archive, with --output
an absent directory. Builds the
adapter itself or consumes a prebuilt adapter with observed build provenance,
then seals every artifact.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from audit_manifest import _source, begin_run, finish_run, verify_manifest

ROOT = Path(__file__).resolve().parents[3]
GNU_REV = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
GNU_SHA = 'a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8'
# These are exact source slices, checked before execution, not verdicts.
FORMS = {
    'ert-test-body-runs': ('lisp/emacs-lisp/ert-tests.el',
        '(ert-deftest ert-test-body-runs ()\n  (setq ert--test-body-was-run t))',
        '(ert-deftest ert-test-body-runs () nil)'),
    'ert-test-deftest-lexical-binding-t': ('lisp/emacs-lisp/ert-tests.el',
        '(ert-deftest ert-test-deftest-lexical-binding-t ()\n'
        '  "Check that `lexical-binding\' in `ert-deftest\' has the file value."\n'
        '  (should (equal lexical-binding t)))', None),
    'ert-test-should-error-argument': ('lisp/emacs-lisp/ert-tests.el',
        '(ert-deftest ert-test-should-error-argument ()\n'
        '  "Errors due to evaluating arguments should not break tests."\n'
        '  (should-error (identity (/ 1 0))))',
        '(ert-deftest ert-test-should-error-argument () (should-error (identity nil)))'),
    'format-c-float': ('src/editfns-tests.el',
        '(ert-deftest format-c-float ()\n  (should-error (format "%c" 0.5)))',
        '(ert-deftest format-c-float () (should-error nil))'),
}


def parse_vm(text, name):
    events = [json.loads(line) for line in text.splitlines() if line.startswith('{')]
    failures = [row for row in events if row.get('ok') is False]
    if failures:
        raise ValueError('VM setup or registration failed: ' + str(failures))
    done = [row for row in events if row.get('event') == 'done']
    if len(done) != 1 or done[0].get('registered') != 1:
        raise ValueError('missing or inconsistent VM completion')
    results = [row for row in events if row.get('event') == 'result']
    if len(results) != 1 or results[0].get('test') != name:
        raise ValueError('missing or inconsistent VM identity')
    if results[0].get('expected') != ':passed':
        raise ValueError('VM expected result is not :passed')
    if results[0].get('outcome') not in ('passed', 'failed'):
        raise ValueError('VM did not complete an assertion outcome')
    return results[0]


def parse_gnu(text, name):
    rows = [line.split('|') for line in text.splitlines() if line.startswith('VCHAIN|')]
    if len(rows) != 1 or len(rows[0]) != 6 or rows[0][1] != name:
        raise ValueError('missing or inconsistent GNU completion/identity')
    _, _, outcome, total, expected, unexpected = rows[0]
    if total != '1' or (expected, unexpected) not in (('1', '0'), ('0', '1')):
        raise ValueError('inconsistent GNU batch counts')
    if outcome not in ('passed', 'failed'):
        raise ValueError('GNU did not complete an assertion outcome')
    if (expected, unexpected) != (('1', '0') if outcome == 'passed' else ('0', '1')):
        raise ValueError('GNU outcome disagrees with batch counts')
    return {'test': name, 'outcome': outcome, 'total': int(total),
            'expected': int(expected), 'unexpected': int(unexpected)}


def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--subject-only', action='store_true',
                    help='reviewed subject-only recipes with positive and negative controls')
    ap.add_argument('--binary', type=Path, help='prebuilt adapter; requires --build-receipt')
    ap.add_argument('--build-receipt', type=Path, help='observed build receipt for --binary')
    args = ap.parse_args()
    if (args.binary is None) != (args.build_receipt is None):
        ap.error('--binary and --build-receipt must be supplied together')
    output = args.output.absolute()
    if output.exists():
        raise ValueError('output must not exist; never rebind a prior run')
    oracle = Path(os.environ['ROMACS_GNU_ORACLE']).resolve(strict=True)
    if hashlib.sha256(oracle.read_bytes()).hexdigest() != GNU_SHA:
        raise ValueError('GNU binary does not match required pin')
    for rel, original, _ in FORMS.values():
        if original not in (ROOT / 'test/gnu/test' / rel).read_text():
            raise ValueError(f'exact source slice changed: {rel}')
    source = _source(ROOT)
    # Separate build settlement from the audit directory's empty-start guard.
    build_dir = output.with_name(output.name + '-build')
    build_dir.mkdir(parents=True, exist_ok=False)
    binary = build_dir / 'romacs-gnu-ert'
    build_inputs = []
    if args.binary is not None:
        shutil.copy2(args.binary.resolve(strict=True), binary)
        shutil.copy2(args.build_receipt.resolve(strict=True), build_dir / 'receipt.json')
    else:
        build = ['cargo', 'build', '--release', '--locked', '--jobs', '3', '-p', 'romacs-gnu-tests',
                 '--bin', 'romacs-gnu-ert', '--message-format=json']
        with (build_dir / 'stdout').open('w') as stdout, (build_dir / 'stderr').open('w') as stderr:
            built = subprocess.run(build, cwd=ROOT, stdout=stdout, stderr=stderr)
        if built.returncode != 0 or _source(ROOT) != source:
            raise ValueError('build failed or source changed during build')
        messages = [json.loads(line) for line in (build_dir / 'stdout').read_text().splitlines()
                    if line.startswith('{')]
        artifacts = [m['executable'] for m in messages if m.get('reason') == 'compiler-artifact'
                     and m.get('target', {}).get('name') == 'romacs-gnu-ert' and m.get('executable')]
        if len(artifacts) != 1:
            raise ValueError('cargo did not report exactly one adapter executable')
        shutil.copy2(Path(artifacts[0]).resolve(strict=True), binary)
        save(build_dir / 'environment.json', {
            'cargo': subprocess.check_output(['cargo', '-V'], text=True),
            'rustc': subprocess.check_output(['rustc', '-vV'], text=True),
            'environment': {k: v for k, v in os.environ.items()
                            if k.startswith('CARGO_PROFILE_') or k in
                            ('CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET', 'CARGO_BUILD_JOBS',
                             'RUSTFLAGS', 'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
                             'CARGO_ENCODED_RUSTFLAGS', 'PATH', 'HOME')},
            'reported_executable': artifacts[0],
        })
        from vacuity_binding import source_identity
        receipt = {'schema': 1, 'source': source_identity(source),
                   'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                   'command': build, 'exit_code': built.returncode,
                   'tracked_clean_start': True, 'tracked_clean_end': True}
        save(build_dir / 'receipt.json', receipt)
        build_inputs = [build_dir / 'stdout', build_dir / 'stderr',
                        build_dir / 'environment.json']
    harness = ROOT / 'test/gnu/harness/romacs-mini-ert.el'
    cases = []
    commands = []
    if args.subject_only:
        from vacuity_classifier import recipes
        forms = recipes()
    else:
        forms = FORMS
    for name, (rel, original, subject) in forms.items():
        mutations = (('original', 'subject-noop') if subject else ('original',)) if args.subject_only else (
            'original', 'subject-noop', 'whole-body-noop-control')
        for mutation in mutations:
            if mutation == 'original':
                form = original
            elif mutation == 'whole-body-noop-control':
                form = f'(ert-deftest {name} () nil)'
            elif subject is not None:
                form = subject
            else:
                # Preserve the assertion and fault its observed binding.
                # VM refusal to bind this constant is BLOCKED, never a kill.
                form = (f'(ert-deftest {name} () (let ((lexical-binding nil)) '
                        '(should (equal lexical-binding t))))')
            for engine in ('romacs-mini', 'gnu-real', 'gnu-mini'):
                stem = f'{name}--{mutation}--{engine}'
                if engine == 'romacs-mini':
                    path = output / (stem + '.json')
                    command = [str(binary), '--harness', str(harness), '--plan', str(path)]
                else:
                    path = output / (stem + '.el')
                    command = [str(oracle), '-Q', '--batch', '-l', str(path)]
                label = 'observed-binding-nil' if subject is None and mutation == 'subject-noop' else mutation
                cases.append((name, rel, label, engine, form, path, command))
                commands.append(command)
    native_inputs = sorted(p for p in oracle.parent.parent.rglob('*')
                           if p.is_file() and not p.is_symlink())
    start = begin_run(ROOT, audit_dir=output,
        binaries={'gnu': {'path': str(oracle), 'provenance': {'kind': 'native-gnu-pin',
                    'revision': GNU_REV, 'sha256': GNU_SHA}},
                  'romacs': {'path': str(binary), 'provenance': {'kind': 'build-receipt',
                    'path': str(build_dir / 'receipt.json')}}},
        harness_files=[__file__, harness, ROOT / 'test/gnu/tools/audit_manifest.py',
                       ROOT / 'test/gnu/tools/vacuity_classifier.py',
                       *build_inputs,
                       Path(sys.executable).resolve(), *native_inputs], commands=commands)
    observations = []
    artifacts = []
    for name, rel, mutation, engine, form, path, command in cases:
        setup = ['(defvar ert--test-body-was-run nil)']
        if engine != 'romacs-mini':
            setup.insert(0, '(setq lexical-binding t)')
        if engine == 'romacs-mini':
            save(path, {'file': str(ROOT / 'test/gnu/test' / rel),
                        'forms': [{'ordinal': i, 'line': 1, 'source': s}
                                  for i, s in enumerate([*setup, form], 1)]})
        else:
            initial = "(require 'ert)" if engine == 'gnu-real' else f'(load {json.dumps(str(harness))} nil t)'
            if engine == 'gnu-real':
                run = f'''(let* ((stats (ert-run-tests-batch '(member {name})))
 (result (ert-test-most-recent-result (ert-get-test '{name}))))
 (princ (format "VCHAIN|{name}|%s|%s|%s|%s\\n"
 (cond ((ert-test-passed-p result) "passed")
       ((and (ert-test-result-with-condition-p result)
             (eq (car (ert-test-result-with-condition-condition result)) 'ert-test-failed)) "failed")
       (t "errored"))
 (ert-stats-total stats) (ert-stats-completed-expected stats)
 (ert-stats-completed-unexpected stats))))'''
            else:
                run = f'''(romacs-gnu-tests--run 0)
(princ (format "VCHAIN|{name}|%s|1|%s|%s\\n"
(car romacs-gnu-tests--last)
(if (eq (car romacs-gnu-tests--last) 'passed) 1 0)
(if (eq (car romacs-gnu-tests--last) 'passed) 0 1)))'''
            path.write_text('\n'.join([';;; -*- lexical-binding: t -*-', initial, *setup, form, run]))
        # Each process has private HOME/XDG state; these are not source inputs.
        home = output / (path.stem + '-home')
        home.mkdir()
        environ = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                       XDG_CACHE_HOME=str(home / 'cache'))
        result = subprocess.run(command, cwd=ROOT, env=environ, capture_output=True,
                                text=True, timeout=60)
        stdout = path.with_suffix(path.suffix + '.stdout')
        stderr = path.with_suffix(path.suffix + '.stderr')
        stdout.write_text(result.stdout)
        stderr.write_text(result.stderr)
        artifacts.extend(p.name for p in (path, stdout, stderr))
        row = {'file': rel, 'test': name, 'mutation': mutation, 'engine': engine,
               'command': command, 'exit_code': result.returncode,
               'artifact_paths': [path.name, stdout.name, stderr.name],
               'environment': {k: environ[k] for k in ('HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME')}}
        try:
            if result.returncode != 0:
                raise ValueError(f'process exit {result.returncode}')
            row['result'] = (parse_vm(result.stdout, name) if engine == 'romacs-mini'
                             else parse_gnu(result.stdout, name))
        except (ValueError, json.JSONDecodeError) as error:
            row['blocked'] = str(error)
        observations.append(row)
    save(output / 'member-results.json', {'schema': 1,
        'expected_cases': [dict(file=rel, test=name, mutation=mutation, engine=engine)
                           for name, rel, mutation, engine, *_ in cases], 'scope': 'isolated source-extracted deftests and controls, not full-file parity',
        'classification': 'subject-only-v1' if args.subject_only else 'NOT ASSIGNED: historical control',
        'observations': observations})
    artifacts.append('member-results.json')
    finish_run(start, output, result_files=artifacts)
    verify_manifest(output)
    print(json.dumps(observations, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
