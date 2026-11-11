#!/usr/bin/env python3
"""VM2 gate inventory, mirror, completion and receipt regressions; no Cargo build."""
import argparse
import contextlib
import importlib.machinery
import importlib.util
import io
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

loader = importlib.machinery.SourceFileLoader('gate', str(pathlib.Path(__file__).resolve().parents[1] / 'gate'))
spec = importlib.util.spec_from_loader('gate', loader)
assert spec is not None
gate = importlib.util.module_from_spec(spec)
loader.exec_module(gate)


class WorkspaceInventory(unittest.TestCase):
    def test_all_members_are_partitioned_without_duplicates(self):
        names = ['romacs-vm', 'romacs-layout', 'new-support-crate']
        metadata = {'workspace_members': names, 'packages': [dict(id=n, name=n) for n in names]
                    + [dict(id='external', name='external')]}
        suites = gate.workspace_suites(metadata)
        self.assertEqual(suites, {'vm2': ['new-support-crate', 'romacs-vm'],
                                 'display': ['romacs-layout']})
        self.assertEqual(sorted(sum(suites.values(), [])), sorted(names))

    def test_oracle_suites_are_not_excluded(self):
        names = ['romacs-vm', 'romacs-display-runtime',
                 'romacs-video', 'new-vm2-oracle-tests']
        metadata = {'workspace_members': names, 'packages': [dict(id=n, name=n) for n in names]}
        suites = gate.workspace_suites(metadata)
        self.assertEqual(set(sum(suites.values(), [])), set(names))
        self.assertIn('new-vm2-oracle-tests', suites['vm2'])

    def test_missing_vm2_is_refused(self):
        with self.assertRaises(ValueError):
            gate.workspace_suites({'workspace_members': [], 'packages': []})

    def test_unknown_member_is_refused(self):
        with self.assertRaises(ValueError):
            gate.workspace_suites({'workspace_members': ['missing'], 'packages': []})


class Completion(unittest.TestCase):
    def event(self, name='crate::binary$test', result='ok'):
        return dict(type='test', name=name, event=result)

    def test_selected_test_cannot_disappear(self):
        self.assertTrue(gate.test_completion_errors([], set(), {'crate::binary$test'}, 1))

    def test_unlisted_result_is_refused(self):
        self.assertTrue(gate.test_completion_errors([self.event()], set(), set(), 1))

    def test_duplicate_result_is_refused(self):
        self.assertTrue(gate.test_completion_errors([self.event(), self.event()], set(),
                                                   {'crate::binary$test'}, 1))

    def test_one_result_per_selected_test(self):
        self.assertEqual(gate.test_completion_errors([self.event()], set(),
                                                    {'crate::binary$test'}, 1), [])

    def test_started_without_terminal_is_refused(self):
        self.assertTrue(gate.test_completion_errors([self.event(result='started')], set(),
                                                   {'crate::binary$test'}, 1))

    def test_ignored_inventory_needs_no_body_result(self):
        self.assertEqual(gate.test_completion_errors([], {'crate::binary$test'}, set(), 1), [])

    def test_stress_iterations_are_distinct(self):
        events = [self.event(f'crate::binary@stress-{i}$test') for i in range(2)]
        self.assertEqual(gate.test_completion_errors(events, set(), {'crate::binary$test'}, 2), [])
        self.assertTrue(gate.test_completion_errors(events[:1], set(), {'crate::binary$test'}, 2))


class Projection(unittest.TestCase):
    def test_real_rsync_removes_previous_runtime_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source, dest = root / 'source', root / 'dest'
            source.mkdir()
            (source / 'Cargo.toml').write_text('workspace')
            (dest / 'lisp').mkdir(parents=True)
            (dest / 'lisp/old.elc').write_bytes(b'obsolete')
            gate.sync(source, dest)
            self.assertEqual(gate.mirror_files(source), gate.mirror_files(dest))
            self.assertFalse((dest / 'lisp').exists())
            self.assertEqual((dest / 'Cargo.toml').read_text(), 'workspace')


class Receipts(unittest.TestCase):
    def setUp(self):
        self.enterContext(mock.patch.object(gate, '_pinned_oracle', return_value='/fixture/oracle'))

    def test_full_writes_green_receipt_with_exact_passed_test(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            cache = root / 'cache'
            cache.mkdir()
            args = argparse.Namespace(oracle=None, worktree=str(root), run_dir=str(root / 'run'),
                                      tree='frozen-tree', group='nocore', mode='full',
                                      expr='all()', threads=1, display_threads=1,
                                      repeat=1, require=['romacs-vm::romacs_vm$probe'])

            def completed(src, run_dir, expr, threads, run, t0, suites, repeat):
                suites['vm2'] = ['romacs-vm']
                (run_dir / 'vm2.jsonl').write_text(json.dumps(dict(
                    type='test', event='ok', name='romacs-vm::romacs_vm$probe')) + '\n')
                return []

            with mock.patch.object(gate, 'CACHE', cache), \
                 mock.patch.object(gate, 'out', return_value=b''), \
                 mock.patch.object(gate, 'sh'), mock.patch.object(gate, 'sync'), \
                 mock.patch.object(gate, 'environment_identity', return_value=('fp', {}, {})), \
                 mock.patch.object(gate, 'build_and_run', side_effect=completed), \
                 mock.patch.object(gate, 'digest', return_value='identity'), \
                 contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(gate.inner(args), 0)
            receipt = json.loads((root / 'run/receipt.json').read_text())
            self.assertEqual(receipt['tree'], 'frozen-tree')
            self.assertEqual(receipt['verdict'], 'GREEN')
            self.assertTrue(receipt['complete'])
            self.assertEqual(receipt['passed'], args.require)
            self.assertEqual(receipt['selected'], {'vm2': 1})

    def test_incomplete_build_preserves_failure_reason_in_receipt(self):
        for failure in ('combined build failed (exit 42)', None):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                cache = root / 'cache'
                cache.mkdir()
                args = argparse.Namespace(oracle=None, worktree=str(root), run_dir=str(root / 'run'),
                                          tree='frozen-tree', group='nocore', mode='full',
                                          expr='all()', threads=1, display_threads=1,
                                          repeat=1, require=[])
                def incomplete(src, run_dir, *args):
                    if failure:
                        (run_dir / 'failure-reason.txt').write_text(failure)
                    return None
                with mock.patch.object(gate, 'CACHE', cache), \
                     mock.patch.object(gate, 'out', return_value=b''), \
                     mock.patch.object(gate, 'sh'), mock.patch.object(gate, 'sync'), \
                     mock.patch.object(gate, 'environment_identity', return_value=('fp', {}, {})), \
                     mock.patch.object(gate, 'build_and_run', side_effect=incomplete), \
                     mock.patch.object(gate, 'digest', return_value='identity'), \
                     contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(gate.inner(args), 1)
                receipt = json.loads((root / 'run/receipt.json').read_text())
                self.assertFalse(receipt['complete'])
                self.assertEqual(receipt['reason'], failure or 'build/test execution incomplete')

    def test_failed_input_still_writes_red_receipt_with_tree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            cache = root / 'cache'
            cache.mkdir()
            args = argparse.Namespace(oracle=None, worktree=str(root), run_dir=str(root / 'run'),
                                      tree='frozen-tree', group='nocore', mode='full',
                                      expr='all()', threads=1, display_threads=1,
                                      repeat=1, require=[])
            with mock.patch.object(gate, 'CACHE', cache), \
                 mock.patch.object(gate, 'out', side_effect=ValueError('bad input')), \
                 mock.patch.object(gate, 'digest', return_value='identity'), \
                 contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(gate.inner(args), 1)
            receipt = json.loads((root / 'run/receipt.json').read_text())
            self.assertEqual(receipt['tree'], 'frozen-tree')
            self.assertEqual(receipt['verdict'], 'RED')
            self.assertEqual(receipt['schema'], 2)
            self.assertFalse(receipt['complete'])
            self.assertNotIn('runtime_key', receipt)
            self.assertNotIn('lisp', receipt)

    def test_retired_modes_and_overlay_option_are_rejected(self):
        for args in [['runtime', 'build'], ['runtime-build'], ['packages'], ['oracle-live'],
                     ['baseline'], ['--lisp-source', 'focused', 'all()']]:
            with self.subTest(args=args):
                result = subprocess.run(['python3', str(pathlib.Path(__file__).resolve().parents[1] / 'gate'), *args],
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)


if __name__ == '__main__':
    unittest.main()
