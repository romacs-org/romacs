#!/usr/bin/env python3
"""Behavioral contracts for the verdict-only queue."""
import contextlib
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock
import io
import argparse

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import lanes_queue as q


def load_lanes():
    loader = importlib.machinery.SourceFileLoader('lanes_api', str(HERE / 'lanes'))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class QueueContracts(unittest.TestCase):
    def test_red_ejects_and_regates(self):
        calls = []
        def gate(batch):
            calls.append(tuple(batch))
            return 'bad' not in batch
        result, ejected = q.select_batch(['good', 'bad', 'last'], tuple, gate)
        self.assertEqual(result, ('good', 'last'), 'RED_EJECT: offending tip must not survive')
        self.assertIn(('good', 'last'), calls, 'RED_REGATE: remainder must receive its own gate')
        self.assertEqual(ejected, ['bad'])

    def test_replay_divergence_refuses(self):
        refused = False
        try:
            q.assert_replay_tree('a' * 40, 'b' * 40)
        except q.QueueRefused:
            refused = True
        self.assertTrue(refused, 'TREE_EQUALITY: unequal replay must refuse verdict transfer')

    def test_missing_dependency_refuses(self):
        class API:
            @staticmethod
            def is_ancestor(repo, requirement, master):
                return requirement != 'missing'
        refused = False
        try:
            q.require_dependencies(API, 'repo', 'master', {'coverage': 'missing'})
        except q.QueueRefused:
            refused = True
        self.assertTrue(refused, 'FAIL_CLOSED: absent fail-closed verification must refuse before gating')

    def test_conflict_is_ejected_without_resolution(self):
        def build(tips):
            if 'conflict' in tips:
                raise q.MergeConflict('conflict')
            return tuple(tips)
        result, ejected = q.select_batch(['good', 'conflict'], build, lambda batch: True)
        self.assertEqual(result, ('good',), 'CONFLICT_EJECT: never resolve queue conflicts')
        self.assertEqual(ejected, ['conflict'])

    def test_record_once_after_equality(self):
        events = []
        q.finish_batch('same', 'same', lambda: events.append('prepare'))
        self.assertEqual(events, ['prepare'], 'RECORD_ONCE: prepare final batch exactly once')
        with contextlib.suppress(q.QueueRefused):
            q.finish_batch('gated', 'diverged', lambda: events.append('invalid'))
        self.assertEqual(events, ['prepare'], 'RECORD_AFTER_EQUALITY: never prepare a divergent replay')

    def test_single_green_batch_one_gate(self):
        calls = []
        result, ejected = q.select_batch(['a', 'b'], tuple, lambda b: calls.append(b) or True)
        self.assertEqual(calls, [('a', 'b')], 'ONE_GATE: green batch needs exactly one gate')
        self.assertEqual(result, ('a', 'b'))
        self.assertEqual(ejected, [])

    def test_native_merge_replay_has_no_queue_ancestry(self):
        api = load_lanes()
        with tempfile.TemporaryDirectory(prefix='queue-native-', dir=os.environ.get('TMPDIR')) as directory:
            root = Path(directory)
            repo = root / 'source'
            repo.mkdir()
            api.git(repo, 'init', '-q', '-b', 'master')
            def commit(name):
                (repo / name).write_text(name)
                api.git(repo, 'add', name)
                api.git(repo, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                        '-c', 'commit.gpgsign=false', 'commit', '-q', '-m', name)
                return api.rev(repo, 'HEAD')
            base = commit('base')
            api.git(repo, 'checkout', '--detach', '-q', base)
            a = commit('a')
            api.git(repo, 'checkout', '--detach', '-q', base)
            b = commit('b')
            with q.private_queue(api, repo, root, root / 'receipt') as private:
                gated = q.build_merges(api, private, base, [a, b], 'refs/heads/queue/test')
                replay = q.build_merges(api, private, base, [a, b])
                q.assert_replay_tree(gated['tree'], replay['tree'])
                self.assertFalse(api.is_ancestor(private, gated['commit'], replay['commit']),
                                 'TIP_ONLY: queue commit must not become replay ancestry')
                self.assertEqual(api.rev(repo, 'master'), base, 'MASTER_UNTOUCHED: queue must never publish')
                parents = api.git(private, 'show', '-s', '--format=%P', replay['commit']).stdout.split()
                self.assertEqual(parents[1], b, 'TIP_DELIVERY: replay must take the lane tip directly')
                self.assertEqual(q.tree(api, private, base), q.tree(api, repo, base))
                self.assertEqual(set(api.git(private, 'ls-tree', '--name-only', replay['commit']).stdout.split()),
                                 {'base', 'a', 'b'})

    def test_queue_wires_one_gate_one_preparation_and_dependency_refusal(self):
        api = load_lanes()
        with tempfile.TemporaryDirectory(prefix='queue-flow-', dir=os.environ.get('TMPDIR')) as directory:
            root = Path(directory)
            repo = root / 'source'
            repo.mkdir()
            api.git(repo, 'init', '-q', '-b', 'master')
            (repo / 'scripts').mkdir()
            (repo / 'scripts/gate').write_text('fixture gate boundary')
            def commit(name):
                (repo / name).write_text(name)
                api.git(repo, 'add', '.')
                api.git(repo, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                        '-c', 'commit.gpgsign=false', 'commit', '-q', '-m', name)
                return api.rev(repo, 'HEAD')
            base = commit('base')
            api.git(repo, 'checkout', '--detach', '-q', base)
            a = commit('a')
            api.git(repo, 'checkout', '--detach', '-q', base)
            b = commit('b')
            events = []
            args = argparse.Namespace(repo=str(repo), output=root / 'output', tip=[a, b],
                                      receipt=['a', 'b'], lane='bq', notes=repo, dry_run=True)
            real_git, real_build = api.git, q.build_merges
            def git(repository, *arguments, **kwargs):
                if arguments and arguments[0] == 'verify-commit':
                    return subprocess.CompletedProcess(arguments, 0, '', '')
                return real_git(repository, *arguments, **kwargs)
            def build(*arguments, **kwargs):
                kwargs['signed'] = False
                return real_build(*arguments, **kwargs)
            def gate(api, private, candidate, label, output, sequence):
                events.append(('gate', candidate['tips']))
                return True, dict(verdict='GREEN', outcomes={}), root / 'fixture-run'
            def prepare(api, canonical, notes, commit, output, notes_commit):
                events.append(('prepare', commit))
            with mock.patch.object(api, 'git', git), mock.patch.object(api, 'cmd_check', return_value=0), \
                 mock.patch.object(q, 'validate_candidate_receipt'), mock.patch.object(q, 'run_checked'), \
                 mock.patch.object(q, 'build_merges', build), mock.patch.object(q, 'gate_candidate', gate), \
                 mock.patch.object(q, 'prepare_record', prepare), contextlib.redirect_stdout(io.StringIO()), \
                 contextlib.redirect_stderr(io.StringIO()):
                args.output = repo / 'forbidden-output'
                rc = q.cmd_queue(args, api.Registry(root / 'registry'), api)
                self.assertEqual(rc, 2)
                self.assertFalse(args.output.exists(), 'OUTPUT_GUARD: refuse in-worktree evidence before any writes')
                args.output = root / 'output'
                with mock.patch.object(q, 'REQUIRED_MASTER_COMMITS', {'absent': 'f' * 40}):
                    rc = q.cmd_queue(args, api.Registry(root / 'registry'), api)
                refused = json.loads((args.output / 'receipt.json').read_text())
                self.assertEqual(rc, 2, 'FAIL_CLOSED_WIRING: absent dependency must refuse the whole command')
                self.assertEqual(events, [], 'FAIL_CLOSED_WIRING: dependency refusal must happen before gate admission')
                self.assertFalse(refused['complete'])
                self.assertEqual(refused['schema'], 2)
                args.output = root / 'accepted-output'
                with mock.patch.object(q, 'REQUIRED_MASTER_COMMITS', {}):
                    rc = q.cmd_queue(args, api.Registry(root / 'registry'), api)
            result = json.loads((args.output / 'receipt.json').read_text())
            self.assertEqual(rc, 0, result)
            self.assertEqual([kind for kind, value in events], ['gate', 'prepare'],
                             'BATCH_WIRING: exactly one gate then one record preparation')
            self.assertEqual(result['tree'], result['replay_tree'], 'BATCH_WIRING: replay must equal the gated tree')
            self.assertTrue(result['ephemeral_ref_discarded'])
            self.assertFalse(Path(result['private_repository']).exists())
            self.assertEqual(api.rev(repo, 'master'), base, 'MASTER_UNTOUCHED: successful dry run must not publish')

    def test_schema_rejects_truthy_complete_and_focused_receipts(self):
        receipt = dict(schema=2, verdict='GREEN', complete=True, tree='tree', tier=2,
                       mode='full', expr='all()')
        q.strict_receipt(receipt, 'tree')
        for key, value in [('schema', True), ('complete', 'false'), ('expr', 'test(x)'),
                           ('mode', 'build'), ('tree', 'other')]:
            bad = dict(receipt, **{key: value})
            with self.subTest(key=key), self.assertRaises(q.QueueRefused):
                q.strict_receipt(bad, 'tree')

    def test_kill_cleans_private_ref_without_touching_master(self):
        api = load_lanes()
        with tempfile.TemporaryDirectory(prefix='queue-test-', dir=os.environ.get('TMPDIR')) as directory:
            root = Path(directory)
            repo = root / 'source'
            subprocess.run(['git', 'init', '-q', '-b', 'master', str(repo)], check=True)
            subprocess.run(['git', '-C', str(repo), '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '-q', '--allow-empty', '-m', 'base'], check=True)
            master = api.rev(repo, 'master')
            receipt = root / 'receipt.json'
            receipt.write_text(json.dumps({'schema': 2, 'verdict': 'RED', 'complete': False}))
            ready = root / 'ready'
            program = '''import sys,time,json
from pathlib import Path
sys.path.insert(0,sys.argv[1])
import lanes_queue as q
import lanes_queue_test as t
with q.private_queue(t.load_lanes(),Path(sys.argv[2]),Path(sys.argv[3]),Path(sys.argv[4])) as private:
    api=t.load_lanes()
    api.git(private,'update-ref','refs/heads/queue/test',api.rev(Path(sys.argv[2]),'master'))
    Path(sys.argv[5]).write_text(str(private))
    time.sleep(60)
'''
            child = subprocess.Popen([sys.executable, '-c', program, str(HERE), str(repo), str(root), str(receipt), str(ready)])
            try:
                deadline = time.monotonic() + 10
                while not ready.exists() and child.poll() is None and time.monotonic() < deadline:
                    time.sleep(.02)
                self.assertTrue(ready.exists(), 'kill fixture must reach private ref creation')
                private = Path(ready.read_text())
                child.kill()
                child.wait(timeout=5)
                deadline = time.monotonic() + 10
                while private.exists() and time.monotonic() < deadline:
                    time.sleep(.02)
                self.assertFalse(private.exists(), 'KILL_CLEANUP: SIGKILL must leave no orphaned queue ref')
                self.assertEqual(api.rev(repo, 'master'), master, 'MASTER_UNTOUCHED: interruption must never half-merge master')
                self.assertEqual(api.git(repo, 'for-each-ref', '--format=%(refname)', 'refs/heads/queue').stdout, '')
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait()


if __name__ == '__main__':
    unittest.main()
