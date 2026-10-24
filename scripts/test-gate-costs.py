#!/usr/bin/env python3
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('gate_costs', Path(__file__).with_name('gate-costs.py'))
assert spec is not None and spec.loader is not None
costs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(costs)


class CostsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.receipt = dict(complete=True, verdict='GREEN', mode='full', repeat=1,
                            suites=['display'], selected={'display': 2},
                            passed=['p::b$a', 'p::b$b'], tree='tree', gate_sha='gate', env_fp='env',
                            runner_sha='runner', oracle_sha='oracle', oracle_source='pin',
                            tool_hashes={'cargo': 'cargo-hash'})
        self.inventory = {'rust-suites': {'p::b': {
            'package-name': 'p', 'binary-name': 'b', 'testcases': {
                name: {'ignored': name == 'skip', 'filter-match': {'status': 'matches'}}
                for name in ('a', 'b', 'skip')}}}}
        self.events = [dict(type='test', event='ok', name=f'p::b${name}', exec_time=seconds)
                       for name, seconds in [('a', 1.0), ('b', 2.0)]]
        self.save()

    def save(self):
        for name, value in [('receipt.json', self.receipt), ('display-tests.json', self.inventory)]:
            (self.root / name).write_text(json.dumps(value))
        (self.root / 'display.jsonl').write_text('\n'.join(map(json.dumps, self.events)))
        (self.root / 'display.log').write_text('Summary [ 2.5s] 2 tests run: 2 passed, 1 skipped\n')

    def test_ranking_and_identical_coverage(self):
        run = costs.read_run(self.root)
        self.assertEqual([r['name'] for r in run['costs']], ['p::b$b', 'p::b$a'])
        self.assertEqual(run['inventory']['display']['inventory_ignored'], ['p::b$skip'])
        self.assertTrue(costs.compare(run, copy.deepcopy(run))['identical_test_names'])

    def test_incomplete_receipt_refused(self):
        self.receipt['complete'] = False
        self.save()
        with self.assertRaisesRegex(ValueError, 'complete GREEN'):
            costs.read_run(self.root)

    def test_missing_and_duplicate_terminal_refused(self):
        original = self.events
        for events in [original[:1], original + original[:1]]:
            self.events = events
            self.save()
            with self.assertRaisesRegex(ValueError, 'identities'):
                costs.read_run(self.root)

    def test_same_count_different_name_refused(self):
        run = costs.read_run(self.root)
        changed = copy.deepcopy(run)
        changed['inventory']['display']['selected'][0] = 'p::b$c'
        with self.assertRaisesRegex(ValueError, 'test-name'):
            costs.compare(run, changed)

    def test_ignored_membership_refused(self):
        run = costs.read_run(self.root)
        changed = copy.deepcopy(run)
        changed['inventory']['display']['inventory_ignored'] = ['p::b$other']
        with self.assertRaisesRegex(ValueError, 'test-name'):
            costs.compare(run, changed)

    def test_changed_environment_or_gate_refused(self):
        run = costs.read_run(self.root)
        for key in ('env_fp', 'gate_sha', 'runner_sha', 'oracle_sha', 'oracle_source', 'tool_hashes'):
            changed_receipt = copy.deepcopy(self.receipt)
            changed_receipt[key] = {'cargo': 'changed'} if key == 'tool_hashes' else 'changed'
            (self.root / 'receipt.json').write_text(json.dumps(changed_receipt))
            changed = costs.read_run(self.root)
            with self.assertRaisesRegex(ValueError, key):
                costs.compare(run, changed)

    def test_nonfinite_duration_refused(self):
        self.events[0]['exec_time'] = float('nan')
        self.save()
        with self.assertRaisesRegex(ValueError, 'duration'):
            costs.read_run(self.root)


if __name__ == '__main__':
    unittest.main()
