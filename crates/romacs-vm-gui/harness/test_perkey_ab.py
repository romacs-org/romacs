#!/usr/bin/env python3
"""perkey_ab's verdicts and report, without a compositor or binary."""
import unittest

import perkey_ab as ab


class Verdicts(unittest.TestCase):
    def test_one_binary_against_itself_is_flat_and_noisy(self):
        counts = {'self-insert': [5.0e6, 5.1e6, 4.9e6], 'C-n': [3.0e6, 3.0e6, 3.1e6]}
        report = ab.compare(counts, counts, 2.0)
        for phase in report.values():
            self.assertEqual(phase['delta_pct'], 0)
            self.assertTrue(phase['noisy'])
            self.assertEqual(phase['verdict'], 'flat or better')
        self.assertEqual(ab.exit_code(report), 0)

    def test_a_rise_past_the_bound_beyond_both_ranges_is_a_regression(self):
        report = ab.compare({'C-f': [100, 101, 102]}, {'C-f': [110, 111, 112]}, 2.0)
        self.assertFalse(report['C-f']['noisy'])
        self.assertEqual(report['C-f']['verdict'], 'regression')
        self.assertEqual(ab.exit_code(report), 1)

    def test_a_rise_past_the_bound_inside_the_scatter_is_inconclusive(self):
        report = ab.compare({'C-f': [100, 120, 101]}, {'C-f': [105, 104, 121]}, 2.0)
        self.assertTrue(report['C-f']['noisy'])
        self.assertEqual(report['C-f']['verdict'], 'inconclusive')
        self.assertEqual(ab.exit_code(report), 2)

    def test_a_fall_is_flat_or_better(self):
        lower = ab.compare({'C-n': [100, 101, 102]}, {'C-n': [90, 91, 92]}, 2.0)
        self.assertLess(lower['C-n']['delta_pct'], 0)
        self.assertEqual(lower['C-n']['verdict'], 'flat or better')
        self.assertEqual(ab.exit_code(lower), 0)

    # A rise under the bound: what it takes for it to count.

    def test_a_steady_rise_under_the_bound_is_a_regression(self):
        # +0.6% in every pair, each arm's spread under 0.1%.
        report = ab.compare({'self-insert': [5.000e6, 5.002e6, 4.998e6]},
                            {'self-insert': [5.030e6, 5.032e6, 5.029e6]}, 2.0)
        phase = report['self-insert']
        self.assertLess(phase['delta_pct'], 2.0)
        self.assertEqual((phase['verdict'], phase['reason']), ('regression', 'consistent rise'))
        self.assertEqual(ab.exit_code(report), 1)
        self.assertIn('regression (consistent rise)', ab.table(report, 2.0, 3))

    def test_pairs_straddling_zero_are_flat(self):
        report = ab.compare({'C-n': [1000, 1000, 1000]}, {'C-n': [1005, 998, 1004]}, 2.0)
        self.assertEqual(report['C-n']['verdict'], 'flat or better')
        self.assertEqual(ab.exit_code(report), 0)

    def test_a_rise_in_every_pair_inside_the_spread_is_flat(self):
        # Every pair rose and the arms do not overlap, but the median delta
        # (+0.25%) is under 1.5 times the wider spread (0.2%, so 0.3%).
        report = ab.compare({'C-f': [1000, 1002, 1001]}, {'C-f': [1002.5, 1003, 1004]}, 2.0)
        phase = report['C-f']
        self.assertTrue(min(phase['delta_range_pct']) > 0 and not phase['noisy'])
        self.assertAlmostEqual(phase['rise_floor_pct'], 0.3, places=3)
        self.assertEqual(phase['verdict'], 'flat or better')

    def test_a_rise_in_every_pair_past_the_floor_whose_arms_overlap_is_flat(self):
        # +1.2% in four pairs of five, past 1.5 times the wider spread (0.69%),
        # but one candidate session reads below one base session.
        report = ab.compare({'C-f': [1000, 1000, 1000, 1000, 1006]}, {'C-f': [1012, 1012, 1012, 1005, 1012]}, 2.0)
        phase = report['C-f']
        self.assertTrue(min(phase['delta_range_pct']) > 0 and phase['delta_pct'] > phase['rise_floor_pct'])
        self.assertTrue(phase['noisy'])
        self.assertEqual(phase['verdict'], 'flat or better')

    def test_a_steady_rise_over_too_few_pairs_is_inconclusive(self):
        report = ab.compare({'C-n': [5.000e6, 5.002e6]}, {'C-n': [5.030e6, 5.032e6]}, 2.0)
        self.assertEqual(report['C-n']['verdict'], 'inconclusive')
        self.assertEqual(ab.exit_code(report), 2)

    def test_deltas_pair_sessions_in_order_and_skip_uncounted_pairs(self):
        report = ab.compare({'C-n': [100, None, 200, 100]}, {'C-n': [110, 150, 220, None]}, 50.0)
        self.assertEqual(report['C-n']['pairs'], 2)
        self.assertEqual(report['C-n']['delta_range_pct'], [10.0, 10.0])

    def test_a_phase_with_fewer_than_two_counted_pairs_is_inconclusive(self):
        report = ab.compare({'self-insert': [100, None, 100], 'C-n': [1, 1]},
                            {'self-insert': [100, 100, None], 'C-n': [1, 1]}, 2.0)
        self.assertEqual(report['self-insert']['verdict'], 'inconclusive')
        self.assertEqual(ab.exit_code(report), 2)

    def test_a_count_not_positive_after_idle_does_not_count(self):
        report = ab.compare({'C-n': [100, 0, -5, 100]}, {'C-n': [101, 100, 100, 0]}, 2.0)
        self.assertEqual(report['C-n']['pairs'], 1)
        self.assertEqual(report['C-n']['verdict'], 'inconclusive')

    def test_each_row_names_its_own_pair_count_and_uncounted_phases_keep_a_row(self):
        report = ab.compare({'self-insert': [100, 100, 100], 'C-n': [100, None, 100], 'C-f': [None, None, None]},
                            {'self-insert': [100, 100, 100], 'C-n': [100, 100, 100], 'C-f': [100, 100, 100]}, 2.0)
        rows = {line.split()[0]: line for line in ab.table(report, 2.0, 3).splitlines()[1:]}
        self.assertIn('3 pairs', rows['self-insert'])
        self.assertIn('2 pairs', rows['C-n'])
        self.assertIn('0 pairs counted: inconclusive (fewer than 2 pairs)', rows['C-f'])
        self.assertIsNone(report['C-f']['rise_floor_pct'])
        # The floor that decided a close result is printed, not left to the
        # rounded spreads.
        self.assertIn('floor 0.00%', rows['self-insert'])

    def test_a_phase_one_arm_lacks_is_inconclusive(self):
        report = ab.compare({'region': [1, 1]}, {}, 2.0)
        self.assertEqual(report['region']['verdict'], 'inconclusive')
        self.assertEqual(ab.exit_code({}), 2)


class Wall(unittest.TestCase):
    def test_wall_time_is_printed_with_the_load_beside_the_verdict(self):
        walls = {'base': {'C-n': {'p50_ms': 9.5, 'p95_ms': 14.0}},
                 'candidate': {'C-n': {'p50_ms': 9.1, 'p95_ms': 13.2}, 'C-f': {'p50_ms': 7.0, 'p95_ms': 9.0}}}
        text = ab.wall_table(walls, {'start': 41.3, 'end': 55.0})
        self.assertIn('information only', text)
        self.assertIn('load 41.3 -> 55.0', text)
        self.assertIn('base 9.50 / 14.00', text)
        self.assertIn('base -', text.splitlines()[-1])


class Options(unittest.TestCase):
    def test_each_arm_gets_its_binary_its_output_and_the_shared_options(self):
        args, arms = ab.arguments(['--base', '/b', '--binary', '/c', '--output', '/tmp/ab', '--keys', '20'])
        self.assertEqual((str(arms['base'].binary), str(arms['candidate'].binary)), ('/b', '/c'))
        self.assertEqual([arm.keys for arm in arms.values()], [20, 20])
        self.assertNotEqual(arms['base'].output, arms['candidate'].output)

    def test_session_counts_and_profiling_are_refused_in_either_spelling(self):
        for extra in (['--sessions', '3'], ['--sessions=3'], ['--sess=3'], ['--sessions', '5'], ['--sessions', '-1'],
                      ['--sessions', '1'], ['--latency-sessions=1'], ['--latency-s=-1'], ['--latency-sessions', '2'],
                      ['--latency-sessions=2'], ['--latency-s=2'],
                      ['--record', 'C-n'], ['--record=self-insert'], ['--rec', 'C-f']):
            with self.assertRaises(SystemExit, msg=extra):
                ab.arguments(['--base', '/b', '--binary', '/c', '--output', '/tmp/ab'] + extra)


if __name__ == '__main__':
    unittest.main()
