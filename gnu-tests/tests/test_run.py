#!/usr/bin/env python3
"""The GNU-suite runner's own tests.  Run by the gate through
gnu-tests/runner/tests/python_contract.rs, or by hand:

    ROMACS_GNU_ERT=<target>/release/romacs-gnu-ert python3 gnu-tests/tests/test_run.py -v

What each class protects, and the mutant that proves it can fail, is listed in
gnu-tests/tests/MUTANTS.md (replayed by gnu-tests/tests/mutants.py).
"""
from __future__ import annotations

import ast
import contextlib
import io
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import run  # noqa: E402

FIXTURES = HERE.parent / 'runner' / 'tests' / 'fixtures'
BIN = os.environ.get('ROMACS_GNU_ERT')


def gnu_result(outcome, as_expected=True):
    return {'outcome': outcome, 'as_expected': as_expected, 'expected': ':passed', 'tags': [], 'detail': None}


def romacs_result(outcome, detail=None):
    # `caught': the should-error audit ran and found no absent-machinery catch.
    return {'outcome': outcome, 'detail': detail, 'caught': []}


class LoadPath(unittest.TestCase):
    def test_core_owner_precedes_recursive_shadow_and_keeps_other_directories(self):
        with tempfile.TemporaryDirectory(prefix='gts-load-path-') as tmp:
            root = pathlib.Path(tmp)
            lisp = root / 'gnu-lisp'
            directories = [lisp, lisp / 'emacs-lisp', lisp / 'cedet',
                           lisp / 'cedet/semantic', lisp / 'cedet/semantic/debug',
                           lisp / 'textmodes']
            for directory in directories:
                directory.mkdir(parents=True, exist_ok=True)
            for relative in ['emacs-lisp/debug.el', 'cedet/semantic/debug.el',
                             'textmodes/extra-owner.el']:
                (lisp / relative).write_bytes((run.ROOT / 'gnu-lisp/emacs-lisp/debug.el').read_bytes())
            with patch.object(run, 'ROOT', root):
                paths = run.load_path()
            self.assertEqual(paths[:2], [str(lisp), str(lisp / 'emacs-lisp')])
            self.assertEqual(len(paths), len(set(paths)))
            self.assertEqual(set(paths), set(map(str, directories)))
            self.assertEqual(paths[2:], sorted(set(map(str, directories)) - set(paths[:2])))
            resolve = lambda name: next(pathlib.Path(p) / name for p in paths
                                        if (pathlib.Path(p) / name).is_file())
            self.assertEqual(resolve('debug.el'), lisp / 'emacs-lisp/debug.el')
            self.assertEqual(resolve('extra-owner.el'), lisp / 'textmodes/extra-owner.el')

    def test_missing_core_directory_does_not_add_nonexistent_search_path(self):
        with tempfile.TemporaryDirectory(prefix='gts-load-path-') as tmp:
            root = pathlib.Path(tmp)
            (root / 'gnu-lisp/textmodes').mkdir(parents=True)
            with patch.object(run, 'ROOT', root):
                self.assertEqual(run.load_path(), [str(root / 'gnu-lisp'), str(root / 'gnu-lisp/textmodes')])


class Classification(unittest.TestCase):
    """A verdict is `passed` only when romacs really reproduced GNU."""

    def test_agreeing_pass_is_passed(self):
        row = run.classify('t', gnu_result('passed'), gnu_result('passed'), romacs_result('passed'), None, None)
        self.assertEqual((row['verdict'], row['category']), ('passed', None))

    def test_assertion_failure_is_failed_not_passed(self):
        row = run.classify('t', gnu_result('passed'), gnu_result('passed'),
                           romacs_result('failed', '((should nil :value nil))'), None, None)
        self.assertEqual((row['verdict'], row['category']), ('failed', 'failing'))

    def test_error_names_the_missing_function(self):
        row = run.classify('t', gnu_result('passed'), gnu_result('passed'),
                           romacs_result('errored', '(void-function make-list)'), None, None)
        self.assertEqual((row['verdict'], row['category'], row['missing']),
                         ('errored', 'not-yet-implemented', 'make-list'))

    def test_never_run_is_not_yet_runnable(self):
        row = run.classify('t', gnu_result('passed'), gnu_result('passed'), None, 'reader: bad', None)
        self.assertEqual(row['verdict'], 'not-yet-runnable')

    def test_harness_infidelity_blocks_a_romacs_pass(self):
        # The reduced harness, run in GNU, disagrees with real ERT: a romacs
        # "pass" would measure the harness, so it must not count.
        row = run.classify('t', gnu_result('passed'), gnu_result('errored'), romacs_result('passed'), None, None)
        self.assertEqual(row['verdict'], 'not-yet-runnable')

    def test_romacs_skip_of_a_gnu_pass_is_not_passed(self):
        row = run.classify('t', gnu_result('passed'), gnu_result('passed'),
                           romacs_result('skipped', '((skip-unless nil))'), None, None)
        self.assertEqual((row['verdict'], row['category']), ('skipped', 'failing'))

    def test_romacs_pass_of_an_expected_failure_is_failed(self):
        gnu = dict(gnu_result('failed'), expected=':failed')
        row = run.classify('t', gnu, gnu, romacs_result('passed'), None, None)
        self.assertEqual(row['verdict'], 'failed')

    def test_matching_either_gnu_regime_is_parity(self):
        # GNU disagrees with itself: source-loaded fails, byte-compiled passes.
        row = run.classify('t', gnu_result('failed', as_expected=False), gnu_result('passed'),
                           romacs_result('passed'), None, None, gnu_result('passed'))
        self.assertEqual(row['verdict'], 'passed')

    def test_gnu_failure_is_reported_apart(self):
        row = run.classify('t', gnu_result('failed', as_expected=False), gnu_result('failed', False),
                           romacs_result('failed'), None, None)
        self.assertEqual(row['category'], 'gnu-fails-too')

    def test_summary_never_counts_not_runnable_as_passed(self):
        rows = [run.classify('a', gnu_result('passed'), gnu_result('passed'), romacs_result('passed'), None, None),
                run.classify('b', gnu_result('passed'), gnu_result('passed'), None, 'never registered', None),
                run.classify('c', gnu_result('passed'), gnu_result('passed'), romacs_result('errored', 'x'), None, None)]
        for r in rows:
            r['file'] = 'f'
        s = run.summarize(rows, ['f'], {})
        self.assertEqual(s['headline']['passed_on_romacs'], 1)
        self.assertEqual(s['verdicts']['not-yet-runnable'], 1)
        self.assertEqual(s['headline']['runnable_on_romacs'], 2)


PIN_SHA = 'a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8'
DISTRO_SHA = '20a94ee7abc04960e3296deead030a37866d1d1e1522f3efd818838af7fffbe4'


def audited(outcome, caught=(), detail=None):
    """A romacs result whose should-error audit reported CAUGHT (cond, datum) pairs."""
    return dict(romacs_result(outcome, detail), caught=[list(c) for c in caught])


class Honesty(unittest.TestCase):
    """A reduced-harness pass is a pass only if what it caught was real, and a
    GNU answer is GNU's only if the pinned binary gave it."""

    def test_the_pin_digest_is_read_from_parity_reference(self):
        self.assertEqual(run.pinned_gnu_sha256(), PIN_SHA)

    def test_a_distro_recorded_baseline_is_refused(self):
        record = {'gnu': {'emacs': '/usr/bin/emacs-32.0.50', 'sha256': DISTRO_SHA, 'version': 'GNU Emacs 32.0.50'}}
        problem = run.baseline_provenance_problem('src/fns-tests.el', record, PIN_SHA)
        self.assertTrue(problem, 'a baseline recorded by the distro binary was accepted as GNU at the pin')
        for needle in ('gnu-tests/baseline/src/fns-tests.el.json', DISTRO_SHA, PIN_SHA, '/usr/bin/emacs-32.0.50',
                       'run.py baseline --all --force'):
            self.assertIn(needle, problem, 'the refusal must name the baseline, both hashes and the re-record step')
        self.assertIsNone(run.baseline_provenance_problem(
            'src/fns-tests.el', {'gnu': dict(record['gnu'], sha256=PIN_SHA)}, PIN_SHA))
        self.assertTrue(run.baseline_provenance_problem('src/fns-tests.el', {'gnu': {}}, PIN_SHA),
                        'a baseline that names no binary was accepted as GNU at the pin')

    def test_the_committed_distro_baseline_is_refused_by_load_baseline(self):
        recorded = json.loads(run.baseline_path('src/fns-tests.el').read_text())['gnu']['sha256']
        if recorded == PIN_SHA:  # after the owner's re-record the refusal has nothing to refuse
            run.load_baseline('src/fns-tests.el')
            return
        with self.assertRaises(run.Refusal, msg='load_baseline used a baseline recorded by a non-pinned GNU') as cm:
            run.load_baseline('src/fns-tests.el')
        self.assertEqual(cm.exception.status, 3)
        self.assertIn(recorded, str(cm.exception))

    def test_a_should_error_satisfied_by_absent_machinery_is_not_a_pass(self):
        for caught in (('void-function', 'read'), ('void-variable', 'sequence'), ('invalid-function', '')):
            row = run.classify('t', gnu_result('passed'), gnu_result('passed'), audited('passed', [caught]), None, None)
            self.assertNotEqual(row['verdict'], 'passed',
                                f'a should-error satisfied by {caught} was counted as a reduced-harness pass')
            self.assertEqual(row['category'], 'vacuous-pass')
            self.assertIn(caught[0], row['reason'])

    def test_a_catch_pinned_gnu_also_made_is_not_vacuous(self):
        # eval-tests--bug24673 really expects invalid-function; GNU catches it too.
        row = run.classify('t', gnu_result('passed'), gnu_result('passed'),
                           audited('passed', [('invalid-function', '')]), None, None, None,
                           {'gnu_caught': ['invalid-function']})
        self.assertEqual(row['verdict'], 'passed')
        fidelity = dict(gnu_result('passed'), vacuous_void_function=['gts-absent'])
        row = run.classify('t', gnu_result('passed'), fidelity, audited('passed', [('void-function', 'gts-absent')]),
                           None, None)
        self.assertEqual(row['verdict'], 'passed')

    def test_a_pass_with_no_should_error_audit_is_not_counted(self):
        unaudited = {'outcome': 'passed', 'detail': None}
        row = run.classify('t', gnu_result('passed'), gnu_result('passed'), unaudited, None, None)
        self.assertNotEqual(row['verdict'], 'passed', 'a pass whose should-error catches nobody audited was counted')
        self.assertEqual(row['category'], 'unaudited-pass')

    def test_native_variable_catches_match_data_and_multiplicity(self):
        pair = ('void-variable', 'clauses')
        fidelity = dict(gnu_result('passed'), caught=[pair, pair])
        row = run.classify('t', gnu_result('passed'), fidelity,
                           audited('passed', [pair, pair]), None, None)
        self.assertEqual(row['verdict'], 'passed')
        self.assertEqual(row['vacuous_catch'], [])
        fidelity['caught'].pop()
        self.assertEqual(run.vacuous_catches(audited('passed', [pair, pair]), fidelity, None),
                         ['(void-variable clauses)'])
        self.assertEqual(run.vacuous_catches(audited('passed', [('void-variable', 'missing')]), fidelity, None),
                         ['(void-variable missing)'])

    def test_native_missing_setter_does_not_excuse_missing_subject(self):
        fidelity = dict(gnu_result('passed'), caught=[('void-function', '(setf thunk-force)')],
                        vacuous_void_function=['thunk-let'])
        row = run.classify('t', gnu_result('passed'), fidelity,
                           audited('passed', [('void-function', 'thunk-let')]), None, None,
                           evidence={'gnu_caught': ['void-function']})
        self.assertEqual((row['verdict'], row['category']), ('failed', 'vacuous-pass'))
        self.assertEqual(row['missing'], 'thunk-let')

    def test_lossy_native_datum_is_not_an_exact_match(self):
        self.assertEqual(run.vacuous_catches(audited('passed', [('invalid-function', '')]),
                                            {'caught': [('invalid-function', '')]}, None),
                         ['(invalid-function)'])

    def test_a_file_with_a_failed_top_level_form_did_not_load(self):
        plan = {'forms': [{'ordinal': 1, 'deftest': None}, {'ordinal': 2, 'deftest': 't'}]}
        events = [{'event': 'form', 'ordinal': 1, 'ok': False, 'error': "(require 'cl-lib): void"},
                  {'event': 'form', 'ordinal': 2, 'ok': True}]
        run_ = {'events': events, 'results': {'t': audited('passed')}, 'fatal': None}
        baseline = {'real': {'results': {'t': gnu_result('passed')}}, 'mini': {'results': {'t': gnu_result('passed')}},
                    'compiled': {'results': {}}}
        rows, _ = run.classify_file('f.el', baseline, plan, run_, {})
        self.assertEqual(rows[0]['verdict'], 'passed')
        self.assertFalse(rows[0]['file_loaded_clean'],
                         'a file whose top-level require failed was reported as loaded')
        self.assertEqual(rows[0]['failed_forms'], [1])
        events[0] = {'event': 'form', 'ordinal': 1, 'ok': True}
        rows, _ = run.classify_file('f.el', baseline, plan, run_, {})
        self.assertTrue(rows[0]['file_loaded_clean'])
        del events[1]  # a form never reported is not a form that loaded
        rows, _ = run.classify_file('f.el', baseline, plan, run_, {})
        self.assertFalse(rows[0]['file_loaded_clean'], 'a file with an unreported form was reported as loaded')


class NativeCatchAudit(unittest.TestCase):
    def setUp(self):
        self.directory = self.enterContext(tempfile.TemporaryDirectory())
        self.scratch = pathlib.Path(self.directory)
        self.baseline = {'mini': {'results': {'t': gnu_result('passed')}},
                         'real': {'results': {'t': gnu_result('passed')}},
                         'compiled': {'results': {}}}
        self.vm_run = {'results': {'t': audited('passed', [('void-variable', 'clauses')])}}
        self.fresh = dict(exit=0, complete=True, timed_out=False, garbled_lines=0,
                          results={'t': dict(gnu_result('passed'), caught=[['void-variable', 'clauses']])})
        self.enterContext(patch.object(run, 'pinned_oracle', return_value={'path': '/pinned/gnu'}))
        self.native = self.enterContext(patch.object(run, 'gnu_run', return_value=self.fresh))

    def refresh(self):
        return run.refresh_catch_audit('f.el', self.baseline, {'path': 'plan'},
                                      self.vm_run, self.scratch, 30)

    def test_refresh_adds_current_facts_without_mutating_baseline(self):
        updated = self.refresh()
        self.assertNotIn('caught', self.baseline['mini']['results']['t'])
        self.assertEqual(updated['mini']['results']['t']['caught'], [['void-variable', 'clauses']])
        self.assertTrue((self.scratch / 'native-catch-audit.json').is_file())
        self.native.assert_called_once_with('mini', 'f.el', self.scratch, 30, {'path': 'plan'}, '/pinned/gnu')

    def test_incomplete_native_audit_is_refused(self):
        for key, value in [('exit', 1), ('complete', False), ('timed_out', True), ('garbled_lines', 1)]:
            with self.subTest(key=key), patch.dict(self.fresh, {key: value}):
                with self.assertRaises(run.Refusal):
                    self.refresh()

    def test_missing_audit_or_changed_fidelity_is_refused(self):
        for result in [None, gnu_result('passed'), dict(gnu_result('failed'), caught=[])]:
            with self.subTest(result=result), patch.dict(self.fresh, results={'t': result}):
                with self.assertRaises(run.Refusal):
                    self.refresh()

    def test_unrecorded_vm_pass_needs_no_native_audit_and_remains_failed(self):
        for result in (None, run.NOT_RECORDED):
            with self.subTest(result=result):
                self.baseline['real']['results'] = {} if result is None else {'t': result}
                self.assertIs(self.refresh(), self.baseline)
                self.native.assert_not_called()
                plan = {'forms': [{'ordinal': 1, 'deftest': 't'}]}
                vm = dict(self.vm_run, events=[], fatal=None)
                rows, extras = run.classify_file('f.el', self.baseline, plan, vm, {})
                self.assertEqual(extras, [])
                self.assertEqual(len(rows), 1)
                self.assertEqual((rows[0]['verdict'], rows[0]['category']), ('failed', 'gnu-not-recorded'))

    def test_compiled_recording_alone_requires_a_complete_audit(self):
        self.baseline['real']['results'] = {'t': run.NOT_RECORDED}
        self.baseline['compiled']['results'] = {'t': gnu_result('passed')}
        self.fresh['complete'] = False
        with self.assertRaises(run.Refusal):
            self.refresh()
        self.native.assert_called_once()

    def test_mixed_recording_never_forgives_an_incomplete_eligible_audit(self):
        self.vm_run['results']['unrecorded'] = audited('passed', [('void-function', 'missing')])
        self.fresh['complete'] = False
        with self.assertRaises(run.Refusal):
            self.refresh()
        self.native.assert_called_once()

    def test_failed_audit_is_preserved_in_output_before_cmd_run_refuses(self):
        import argparse
        output = self.scratch / 'output'
        self.fresh['complete'] = False
        args = argparse.Namespace(harness='mini', harness_fidelity=False, engine='romacs', all=False, files=['f.el'],
                                  out=str(output), bin='/bin/true', jobs=1,
                                  test_timeout=1, setup_timeout=1)
        with patch.object(run, 'verify_pin', return_value={}), \
                patch.object(run, 'normalize', return_value='f.el'), \
                patch.object(run, 'load_baseline', return_value=self.baseline), \
                patch.object(run, 'ScratchTree', return_value=contextlib.nullcontext()), \
                patch.object(run, 'extract', return_value={'forms': []}), \
                patch.object(run, 'load_ledger', return_value={}), \
                patch.object(run, 'load_member_evidence', return_value={}), \
                patch.object(run, 'run_romacs', return_value=self.vm_run):
            with self.assertRaisesRegex(run.Refusal, 'incomplete pinned GNU catch audit'):
                run.cmd_run(args)
        self.assertEqual(json.loads((output / 'log' / 'f.el.native-catch-audit.json').read_text()), self.fresh)

    def test_no_void_catch_needs_no_native_rerun(self):
        self.vm_run['results']['t']['caught'] = []
        self.assertIs(self.refresh(), self.baseline)
        self.native.assert_not_called()


VALID_ROW = '''
[[exclude]]
file = "src/editfns-tests.el"
test = "format-properties"
category = "intentional-divergence"
divergence = "VM2-FMT1"
symbol = "format"
gnu = "GNU parses flags, width and precision in format conversions such as %4s."
romacs = "VM2 accepts only bare %s %S %d and %% and signals for anything else."
why = "Fixture row for the runner's own tests; it is never committed to EXCLUSIONS.toml."
gnu_source = "src/editfns.c:3611-3653"
gnu_source_sha256 = "076363ec5358bbcd51f87c205180123b3063a3f38d22b3b057047a1c7a87423f"
verified_by = "Read editfns.c:3611-3653 at the pin; it parses flags, width and precision."
'''

# D001 is listed in SUSPECT-DIVERGENCES.toml: its GNU side is wrong.
SUSPECT_ROW = '''
[[exclude]]
file = "src/data-tests.el"
test = "data-aset-string"
category = "intentional-divergence"
divergence = "D001"
symbol = "aset"
gnu = "GNU refuses to store a non-ASCII character into a multibyte string slot."
romacs = "VM2 splices any character into any multibyte string slot, resizing it."
why = "Fixture row for the runner's own tests; it is never committed to EXCLUSIONS.toml."
gnu_source = "src/data.c:2665-2673"
gnu_source_sha256 = "42a9b2db85756cfa9f8736ec8cc9cab7a7a7b3964aed9993d128a12c3bc25f95"
verified_by = "Read data.c:2665-2673 at the pin: multibyte aset rejects non-ASCII."
'''
KEY = ('src/editfns-tests.el', 'format-properties')


class Ledger(unittest.TestCase):
    """An exclusion exists only as a fully justified row; anything less refuses."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix='gts-ledger-test-')
        with run.ScratchTree():
            cls.plans = {f: run.extract(f, pathlib.Path(cls.tmp.name))
                         for f in ('src/editfns-tests.el', 'src/data-tests.el')}

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def ledger(self, text):
        path = pathlib.Path(self.tmp.name) / 'EXCLUSIONS.toml'
        path.write_text(text)
        return path

    def refused(self, text):
        with self.assertRaises(run.Refusal) as ctx:
            run.load_ledger(self.ledger(text), self.plans)
        self.assertEqual(ctx.exception.status, 2)
        return str(ctx.exception)

    def test_a_justified_verified_row_is_accepted(self):
        rows = run.load_ledger(self.ledger(VALID_ROW), self.plans)
        self.assertEqual(list(rows), [KEY])
        self.assertTrue(rows[KEY]['verified'], rows[KEY]['unverified_because'])

    def test_a_row_citing_a_suspect_entry_is_unverified_debt(self):
        rows = run.load_ledger(self.ledger(SUSPECT_ROW), self.plans)
        row = rows[('src/data-tests.el', 'data-aset-string')]
        self.assertFalse(row['verified'])
        out = run.classify('data-aset-string', gnu_result('passed'), gnu_result('passed'),
                           romacs_result('failed'), None, row)
        self.assertEqual((out['verdict'], out['category']), ('failed', 'excluded-unverified-row'))

    def test_a_citation_that_is_not_the_verified_code_is_refused(self):
        self.assertIn('the cited GNU code is not what was verified',
                      self.refused(VALID_ROW.replace('src/editfns.c:3611-3653', 'src/editfns.c:3611-3654')))

    def test_a_non_implementation_citation_is_refused(self):
        self.assertIn('gnu_source must cite GNU implementation',
                      self.refused(VALID_ROW.replace('src/editfns.c:3611-3653', 'etc/NEWS:1-2')))

    def test_an_unknown_divergence_is_refused(self):
        self.assertIn('not an entry in DIVERGENCES.md', self.refused(VALID_ROW.replace('"VM2-FMT1"', '"D999"')))

    def test_a_missing_justification_is_refused(self):
        text = '\n'.join(l for l in VALID_ROW.splitlines() if not l.startswith('why'))
        self.assertIn('missing or empty `why`', self.refused(text))

    def test_a_one_word_justification_is_refused(self):
        self.assertIn('must explain', self.refused(VALID_ROW.replace(
            'why = "Fixture row for the runner\'s own tests; it is never committed to EXCLUSIONS.toml."',
            'why = "intentional"')))

    def test_a_symbol_the_divergence_does_not_cover_is_refused(self):
        self.assertIn('is not one of the symbols', self.refused(VALID_ROW.replace('symbol = "format"',
                                                                                 'symbol = "car"')))

    def test_a_symbol_the_test_never_uses_is_refused(self):
        row = VALID_ROW.replace('"VM2-FMT1"', '"VM2-PATH1"').replace('symbol = "format"',
                                                                     'symbol = "expand-file-name"')
        self.assertIn('never mentions', self.refused(row))

    def test_a_test_the_file_does_not_define_is_refused(self):
        self.assertIn('no top-level deftest', self.refused(VALID_ROW.replace('"format-properties"',
                                                                             '"format-no-such-test"')))

    def test_a_glob_is_refused(self):
        self.assertIn('no globs', self.refused(VALID_ROW.replace('"format-properties"', '"format-*"')))

    def test_the_only_categories_are_divergence_and_scope(self):
        self.assertIn('category must be one of', self.refused(
            VALID_ROW.replace('"intentional-divergence"', '"flaky"')))

    def test_an_excluded_test_that_passes_is_refused(self):
        rows = run.load_ledger(self.ledger(VALID_ROW), self.plans)
        with self.assertRaises(run.Refusal):
            run.classify('format-properties', gnu_result('passed'), gnu_result('passed'),
                         romacs_result('passed'), None, rows[KEY])

    def test_an_exclusion_moves_a_failure_to_its_category_but_never_to_passed(self):
        rows = run.load_ledger(self.ledger(VALID_ROW), self.plans)
        row = run.classify('format-properties', gnu_result('passed'), gnu_result('passed'),
                           romacs_result('failed'), None, rows[KEY])
        self.assertEqual((row['verdict'], row['category'], row['excluded_by']),
                         ('failed', 'intentional-divergence', 'VM2-FMT1'))

    def test_the_committed_ledger_is_valid(self):
        run.load_ledger(run.LEDGER)


class Pin(unittest.TestCase):
    """The import is GNU's test/ byte for byte, or nothing runs."""

    def test_the_committed_import_is_the_pin(self):
        p = run.verify_pin()
        self.assertEqual(p['gnu_commit'], '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea')

    def test_git_tree_id_agrees_with_git(self):
        with tempfile.TemporaryDirectory(prefix='gts-pin-test-') as tmp:
            root = pathlib.Path(tmp)
            (root / 'd' / 'e').mkdir(parents=True)
            (root / 'd' / 'a.el').write_text('(a)\n')
            (root / 'd' / 'e' / 'b').write_bytes(b'\xff\x00')
            (root / 'd' / 'x.sh').write_text('#!/bin/sh\n')
            (root / 'd' / 'x.sh').chmod(0o755)
            (root / 'd' / 'a-b.el').write_text('x')
            (root / 'd' / 'a').mkdir()
            (root / 'd' / 'a' / 'z').write_text('z')
            subprocess.run(['git', 'init', '-q', str(root)], check=True)
            subprocess.run(['git', '-C', str(root), 'add', '-A'], check=True)
            want = subprocess.run(['git', '-C', str(root), 'write-tree', '--prefix=d/'], check=True,
                                  capture_output=True, text=True).stdout.strip()
            self.assertEqual(run.git_tree_id(root / 'd'), want)

    def test_an_edited_import_is_refused(self):
        with tempfile.TemporaryDirectory(prefix='gts-pin-test-') as tmp:
            root = pathlib.Path(tmp)
            (root / 'gnu-tests').mkdir()
            shutil.copy(run.PIN_FILE, root / 'gnu-tests' / 'PIN.json')
            shutil.copytree(run.TESTS, root / 'gnu-tests' / 'test', symlinks=True)
            run.verify_pin(root)
            victim = root / 'gnu-tests' / 'test' / 'src' / 'fns-tests.el'
            victim.write_text(victim.read_text().replace('(should (', '(should-not (', 1))
            with self.assertRaises(run.Refusal) as ctx:
                run.verify_pin(root)
            self.assertEqual(ctx.exception.status, 3)

    def test_a_stray_file_in_the_import_is_refused(self):
        with tempfile.TemporaryDirectory(prefix='gts-pin-test-') as tmp:
            root = pathlib.Path(tmp)
            (root / 'gnu-tests').mkdir()
            shutil.copy(run.PIN_FILE, root / 'gnu-tests' / 'PIN.json')
            shutil.copytree(run.TESTS, root / 'gnu-tests' / 'test', symlinks=True)
            (root / 'gnu-tests' / 'test' / 'src' / 'fns-tests.elc').write_text('compiled')
            with self.assertRaises(run.Refusal):
                run.verify_pin(root)

    def test_the_bootstrap_chain_is_the_census_chain(self):
        tree = ast.parse((run.ROOT / 'scripts' / 'lisp-census.py').read_text())
        census = next(ast.literal_eval(n.value) for n in ast.walk(tree)
                      if isinstance(n, ast.Assign) and any(getattr(t, 'id', None) == 'BOOTSTRAP_CHAIN'
                                                           for t in n.targets))
        self.assertEqual(run.BOOTSTRAP_CHAIN, census)

    def test_ordered_bootstrap_is_explicit_and_separate(self):
        from types import SimpleNamespace
        self.assertIs(run.source_bootstrap_chain(SimpleNamespace()), run.BOOTSTRAP_CHAIN)
        ordered = run.source_bootstrap_chain(SimpleNamespace(bootstrap='ordered'))
        self.assertIn('bindings', ordered, 'MEASUREMENT REGIME: bindings prefix missing')
        self.assertLess(ordered.index('bindings'), ordered.index('emacs-lisp/easy-mmode'),
                        'MEASUREMENT REGIME: bindings macro must precede easy-mmode compilation')
        self.assertEqual(ordered[5:12], [
            'emacs-lisp/pcase', 'emacs-lisp/gv', 'emacs-lisp/inline',
            'custom', 'bindings', 'emacs-lisp/easy-mmode', 'emacs-lisp/cl-lib'])
        self.assertEqual(ordered[12:], [
            'faces', 'cus-face', 'button',
            'emacs-lisp/debug', 'emacs-lisp/backtrace', 'emacs-lisp/ewoc',
            'emacs-lisp/find-func', 'emacs-lisp/pp', 'emacs-lisp/map', 'help-mode'])
        for harness, engine, fidelity in [('mini', 'romacs', False),
                                          ('real', 'gnu', False),
                                          ('real', 'romacs', True)]:
            with self.assertRaisesRegex(run.Refusal, 'requires --harness real'):
                run.cmd_run(SimpleNamespace(bootstrap='ordered', harness=harness,
                                            engine=engine, harness_fidelity=fidelity))

    def test_the_suite_is_every_gnu_test_file(self):
        self.assertEqual(len(run.suite_files()), 565)


class EndToEnd(unittest.TestCase):
    """Real GNU and the real adapter on fixtures whose answers are known."""

    @classmethod
    def setUpClass(cls):
        # Never skip: a skipped end-to-end class would read as green.
        if not BIN or not os.access(BIN, os.X_OK):
            raise AssertionError(f'ROMACS_GNU_ERT must name the built adapter, not {BIN!r}')

    def rows(self, fixture, extra_args=(), binary=None, test_timeout=60):
        path = str(FIXTURES / fixture)
        with tempfile.TemporaryDirectory(prefix='gts-e2e-') as tmp:
            scratch = pathlib.Path(tmp)
            plan = run.extract(path, scratch)
            baseline = {'real': run.gnu_run('real', path, scratch, 120),
                        'mini': run.gnu_run('mini', path, scratch, 120, plan)}
            log = io.StringIO()
            result = run.run_romacs(pathlib.Path(binary or BIN), path, plan, scratch, test_timeout, 120, log,
                                    extra_args)
            rows, _ = run.classify_file(path, baseline, plan, result, {})
        return {r['test']: r for r in rows}, baseline, result

    def test_adapter_resolves_core_owner_and_remaining_directory(self):
        assert BIN is not None
        with tempfile.TemporaryDirectory(prefix='gts-owner-path-') as tmp:
            root = pathlib.Path(tmp)
            lisp = root / 'gnu-lisp'
            for relative in ['emacs-lisp/debug.el', 'cedet/semantic/debug.el',
                             'textmodes/extra-owner.el']:
                destination = lisp / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                source = 'emacs-lisp/debug.el' if relative.endswith('extra-owner.el') else relative
                shutil.copyfile(run.ROOT / 'gnu-lisp' / source, destination)
            plan = root / 'plan.json'
            plan.write_text(json.dumps({'file': 'owner-resolution', 'forms': []}))
            command = [BIN, '--harness', str(run.HARNESS), '--plan', str(plan)]
            with patch.object(run, 'ROOT', root):
                for directory in run.load_path():
                    command += ['--load-path', directory]
            for library, owner in [('debug', 'emacs-lisp/debug.el'),
                                   ('extra-owner', 'textmodes/extra-owner.el')]:
                command += ['--probe', f'(equal (locate-library "{library}") '
                            f'{json.dumps(str(lisp / owner))})']
            process = subprocess.run(command, capture_output=True, text=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr)
            events = [json.loads(line) for line in process.stdout.split('\n') if line.startswith('{')]
            probes = [event for event in events if event.get('event') == 'probe']
            self.assertEqual(len(probes), 2, process.stdout)
            self.assertEqual([probe['value'] for probe in probes], ['t', 't'], process.stdout)
            self.assertTrue(all(probe.get('error') is None for probe in probes), process.stdout)

    def test_native_variable_catch_audit_preserves_a_real_gnu_member(self):
        assert BIN is not None
        rel, name = 'src/eval-tests.el', 'eval-tests--mutating-cond'
        with run.ScratchTree(), tempfile.TemporaryDirectory(prefix='gts-native-audit-') as tmp:
            scratch = pathlib.Path(tmp)
            plan = run.extract(rel, scratch)
            baseline = run.load_baseline(rel)
            vm_run = run.run_romacs(pathlib.Path(BIN), rel, plan, scratch, 60, 120, io.StringIO())
            updated = run.refresh_catch_audit(rel, baseline, plan, vm_run, scratch, 120)
            pairs = [['void-variable', 'clauses'], ['void-variable', 'clauses']]
            self.assertEqual(updated['mini']['results'][name]['caught'], pairs)
            self.assertEqual(vm_run['results'][name]['caught'], pairs)
            rows, _ = run.classify_file(rel, updated, plan, vm_run, {})
            row = next(row for row in rows if row['test'] == name)
            self.assertEqual((row['verdict'], row['vacuous_catch']), ('passed', []))
            # A different missing variable must still be rejected. This changes
            # only the observed result, never GNU source or its assertions.
            vm_run['results'][name]['caught'][0] = ['void-variable', 'missing-subject']
            rows, _ = run.classify_file(rel, updated, plan, vm_run, {})
            row = next(row for row in rows if row['test'] == name)
            self.assertEqual((row['verdict'], row['category']), ('failed', 'vacuous-pass'))

    def test_each_verdict_is_reported(self):
        rows, baseline, _ = self.rows('runner-fixture-tests.el')
        gnu = {k: v['outcome'] for k, v in baseline['real']['results'].items()}
        self.assertEqual(gnu, {'fixture-passes': 'passed', 'fixture-genuine': 'passed', 'fixture-fails': 'failed',
                               'fixture-errors': 'errored', 'fixture-skips': 'skipped',
                               'fixture-expected-failure': 'failed', 'fixture-wrong-should-error-type': 'failed',
                               'fixture-ert-info-evaluates-its-message': 'errored'})
        got = {k: (v['romacs'], v['verdict']) for k, v in rows.items()}
        self.assertEqual(got['fixture-passes'], ('passed', 'passed'))
        self.assertEqual(got['fixture-genuine'], ('passed', 'passed'))
        self.assertEqual(got['fixture-expected-failure'], ('failed', 'passed'))
        self.assertEqual(got['fixture-fails'][0], 'failed')
        self.assertEqual(got['fixture-errors'][0], 'errored')
        self.assertEqual(got['fixture-skips'][0], 'skipped')
        self.assertEqual(got['fixture-wrong-should-error-type'][0], 'failed')
        self.assertEqual(got['fixture-ert-info-evaluates-its-message'], ('errored', 'errored'))
        self.assertEqual(rows['fixture-fails']['category'], 'gnu-fails-too')

    def test_a_genuine_failure_is_reported_as_failed(self):
        rows, _, _ = self.rows('runner-fixture-tests.el', ('--probe', '(setq gts-fixture-mutant t)'))
        self.assertEqual((rows['fixture-genuine']['romacs'], rows['fixture-genuine']['verdict'],
                          rows['fixture-genuine']['category']), ('failed', 'failed', 'failing'))
        self.assertEqual(rows['fixture-passes']['verdict'], 'passed')

    def test_an_unreadable_file_reports_nothing_as_passed(self):
        assert BIN is not None
        path = str(FIXTURES / 'unreadable-fixture-tests.el')
        names = {'fixture-unreadable-1', 'fixture-unreadable-2'}
        with tempfile.TemporaryDirectory(prefix='gts-e2e-') as tmp:
            scratch = pathlib.Path(tmp)
            plan = run.extract(path, scratch)
            baseline = {'real': run.gnu_run('real', path, scratch, 120),
                        'mini': run.gnu_run('mini', path, scratch, 120, plan)}
            # Records are valid now. Corrupt only the adapter's input after
            # recording the unchanged GNU-positive source, not its verdicts.
            definitions = [form for form in plan['forms'] if form.get('deftest') in names]
            self.assertEqual({form['deftest'] for form in definitions}, names)
            self.assertEqual(len(definitions), 2)
            for form in definitions:
                form['source'] = '#s()'
            pathlib.Path(plan['path']).write_text(json.dumps(plan))
            result = run.run_romacs(pathlib.Path(BIN), path, plan, scratch, 60, 120, io.StringIO())
            rows, _ = run.classify_file(path, baseline, plan, result, {})
        self.assertEqual({k: v['outcome'] for k, v in baseline['real']['results'].items()},
                         {name: 'passed' for name in names})
        self.assertEqual({row['test'] for row in rows}, names)
        self.assertEqual({row['verdict'] for row in rows}, {'not-yet-runnable'})
        self.assertEqual(result['results'], {})
        failures = [event for event in result['events']
                    if event.get('event') == 'form' and not event.get('ok')]
        self.assertEqual({event['ordinal'] for event in failures},
                         {form['ordinal'] for form in definitions})
        self.assertTrue(all('invalid-read-syntax' in event.get('error', '') for event in failures))

    def test_valid_record_literals_remain_runnable(self):
        rows, _, result = self.rows('unreadable-fixture-tests.el')
        self.assertEqual({name: row['verdict'] for name, row in rows.items()},
                         {'fixture-unreadable-1': 'passed', 'fixture-unreadable-2': 'passed'})
        self.assertFalse(result['fatal'])

    def test_an_adapter_that_runs_nothing_reports_nothing_as_passed(self):
        rows, _, result = self.rows('runner-fixture-tests.el', binary='/usr/bin/true')
        self.assertTrue(result['fatal'])
        self.assertEqual({r['verdict'] for r in rows.values()}, {'not-yet-runnable'})

    def test_a_hung_test_times_out_and_later_tests_still_run(self):
        path = str(FIXTURES / 'hang-fixture-tests.el')
        with tempfile.TemporaryDirectory(prefix='gts-e2e-') as tmp:
            scratch = pathlib.Path(tmp)
            plan = run.extract(path, scratch)
            result = run.run_romacs(pathlib.Path(BIN), path, plan, scratch, 3, 120, io.StringIO())
        self.assertEqual(result['results']['fixture-hangs']['outcome'], 'timeout')
        self.assertEqual(result['results']['fixture-after-hang']['outcome'], 'passed')

    def test_a_vacuous_should_error_pass_is_not_counted_end_to_end(self):
        rows, _, result = self.rows('vacuous-fixture-tests.el')
        self.assertEqual({k: v['verdict'] for k, v in rows.items()},
                         {'fixture-should-error-real': 'passed', 'fixture-should-error-typed-void': 'passed',
                          'fixture-no-should-error': 'passed'})
        self.assertEqual(result['results']['fixture-should-error-real']['caught'], [])
        rows, _, result = self.rows('vacuous-fixture-tests.el', ('--probe', '(setq gts-fixture-void t)'))
        self.assertEqual(result['results']['fixture-should-error-real']['caught'],
                         [['void-function', 'gts-fixture-signal']])
        self.assertEqual((rows['fixture-should-error-real']['romacs'], rows['fixture-should-error-real']['verdict'],
                          rows['fixture-should-error-real']['category']), ('passed', 'failed', 'vacuous-pass'),
                         'romacs passed a should-error on void-function where GNU caught a real error')
        self.assertEqual(rows['fixture-should-error-typed-void']['verdict'], 'passed')
        self.assertTrue(all(r['file_loaded_clean'] for r in rows.values()))

    def test_a_gnu_file_runs_end_to_end_with_honest_exit_status(self):
        with tempfile.TemporaryDirectory(prefix='gts-e2e-') as tmp:
            status = run.main(['run', 'src/fns-tests.el', '--bin', BIN, '--out', tmp, '--jobs', '1'])
            if json.loads(run.baseline_path('src/fns-tests.el').read_text())['gnu']['sha256'] != PIN_SHA:
                # Every committed baseline is distro-recorded until the owner's
                # re-record: nothing may be measured against it.
                self.assertEqual(status, 3, 'a run against a distro-recorded baseline was not refused')
                return
            summary = json.loads((pathlib.Path(tmp) / 'summary.json').read_text())
            rows = [json.loads(l) for l in (pathlib.Path(tmp) / 'results.jsonl').read_text().splitlines()]
        self.assertEqual(status, 1, 'fns-tests.el does not fully pass on romacs today')
        self.assertEqual(len(rows), 95)
        self.assertEqual(summary['headline']['gnu_tests'], 95)
        self.assertLess(summary['headline']['passed_on_romacs'], 95)
        self.assertFalse(summary['harness']['is_gnu_ert'])


if __name__ == '__main__':
    unittest.main()
