#!/usr/bin/env python3
"""Ratchet regression tests; synthetic counter mutations are tests, not evidence."""
import copy
import json
import unittest
from read_ratchet import ACCEPTED_RATIOS, FLAT_EVIDENCE, NOISE_ALLOWANCE, checked_costs, qualify


class RatchetTests(unittest.TestCase):
    def setUp(self):
        self.flat = json.loads(FLAT_EVIDENCE.read_text())
        self.good = copy.deepcopy(self.flat)
        for row in self.good['rows']:
            self.set_cost(row['name'], 0.9)
        qualify(self.good)

    def set_cost(self, name, ratio):
        row = next(r for r in self.good['rows'] if r['name'] == name)
        original = next(r for r in self.flat['rows'] if r['name'] == name)
        delta = int(original['costs']['vm2']['median'] * ratio * row['n'])
        for rep in range(3):
            samples = {s['n']:s for s in self.good['samples'] if s['row'] == name and s['engine'] == 'vm2' and s['rep'] == rep}
            samples[2*row['n']]['count'] = samples[row['n']]['count'] + delta
        row['costs']['vm2'] = dict(slopes=[delta/row['n']]*3, median=delta/row['n'])

    def test_each_row_rejects_further_regression(self):
        for name, limit in ACCEPTED_RATIOS.items():
            with self.subTest(row=name):
                self.set_cost(name, limit * NOISE_ALLOWANCE + 0.001)
                with self.assertRaises(AssertionError):
                    qualify(self.good)
                self.set_cost(name, 0.9)

    def test_explicit_trade_is_accepted_but_other_six_require_parity(self):
        for name, limit in ACCEPTED_RATIOS.items():
            self.set_cost(name, limit * NOISE_ALLOWANCE - 0.001)
        qualify(self.good)

    def test_scheduling_answer_summary_and_workload_mutants_fail(self):
        for mutation in ('zero', 'partial', 'answer', 'summary', 'source', 'duplicate'):
            bad = copy.deepcopy(self.good)
            if mutation == 'zero': bad['samples'][0]['count'] = 0
            if mutation == 'partial': bad['samples'][0]['running'] -= 1
            if mutation == 'answer': bad['samples'][0]['stdout'] = 'wrong'
            if mutation == 'summary': bad['rows'][0]['costs']['vm2']['median'] += 1
            if mutation == 'source': bad['source_hashes'] = {}
            if mutation == 'duplicate': bad['samples'][0] = bad['samples'][1]
            with self.subTest(mutation=mutation), self.assertRaises((AssertionError, RuntimeError)):
                qualify(bad)


if __name__ == '__main__':
    unittest.main()
