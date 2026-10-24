#!/usr/bin/env python3
"""Paired bootstrap and sequential repeat contracts."""
import random
import unittest

import bench_stats


class Stats(unittest.TestCase):
    def samples(self, ratio):
        rng = random.Random(43)
        base = [100 * rng.uniform(.7, 1.3) for _ in range(15)]
        cand = [b * ratio * rng.uniform(.97, 1.03) for b in base]
        return base, cand

    def test_known_pass_and_fail_and_boundary(self):
        for ratio, expected in ((1.1, 'PASS'), (1.3, 'FAIL'), (1.2, 'INCONCLUSIVE')):
            base, cand = self.samples(ratio)
            result = bench_stats.paired_stats(cand, base)
            self.assertEqual(result['verdict'], expected)
            self.assertEqual(result['n'], 15)
            self.assertLess(result['ci_lo'], result['median'])
            self.assertGreater(result['ci_hi'], result['median'])

    def test_pair_ratios_not_ratio_of_medians(self):
        result = bench_stats.paired_stats([1, 100, 200], [1, 2, 200])
        self.assertEqual(result['median'], 1)

    def test_fixed_seed_and_inclusive_pass_boundary(self):
        base, cand = self.samples(1.2)
        self.assertEqual(bench_stats.paired_stats(cand, base), bench_stats.paired_stats(cand, base))
        self.assertEqual(bench_stats.paired_stats([1.2] * 7, [1] * 7)['verdict'], 'PASS')

    def test_invalid_or_unpaired_data_is_rejected(self):
        for cand, base in (([], []), ([1], [1, 2]), ([1], [0]), ([float('nan')], [1]), ([True], [1])):
            with self.assertRaises(ValueError):
                bench_stats.paired_stats(cand, base)

    def test_sequential_targets_and_explicit_counts(self):
        self.assertEqual(bench_stats.next_repeat_target(7, ['INCONCLUSIVE']), 11)
        self.assertEqual(bench_stats.next_repeat_target(11, ['INCONCLUSIVE']), 15)
        self.assertEqual(bench_stats.next_repeat_target(15, ['INCONCLUSIVE']), 15)
        self.assertEqual(bench_stats.next_repeat_target(7, ['PASS', 'FAIL']), 7)
        self.assertEqual(bench_stats.next_repeat_target(3, ['INCONCLUSIVE'], explicit=True), 3)


if __name__ == '__main__':
    unittest.main()
