"""scaling_live.py's pure parts: exponents, verdicts, overrides and parsing."""
import math
import unittest

import scaling_live as s

PROBE = {'name': 'p', 'n': 100, 'k': 4, 'budget': 1.25, 'known': None, 'gui': False, 'steps': 1, 'stack': False}


def row(work_n, work_kn, **probe):
    return s.verdict({**PROBE, **probe}, {'instructions': work_n}, {'instructions': work_kn})


class Exponent(unittest.TestCase):
    def test_linear_nlogn_and_quadratic(self):
        self.assertAlmostEqual(s.exponent(10**7, 4 * 10**7, 4), 1.0)
        self.assertAlmostEqual(s.exponent(10**7, 16 * 10**7, 4), 2.0)
        n = 500
        nlogn = (4 * n * math.log(4 * n)) / (n * math.log(n))
        self.assertLess(s.exponent(10**7, nlogn * 10**7, 4), s.DEFAULT_BUDGET)


class Verdict(unittest.TestCase):
    def test_within_budget_passes(self):
        self.assertEqual(row(10**7, 4 * 10**7)['status'], 'PASS')

    def test_quadratic_fails_with_its_exponent(self):
        r = row(10**7, 16 * 10**7)
        self.assertEqual((r['status'], r['exponent']), ('FAIL', 2.0))
        self.assertIn('over budget', r['reason'])

    def test_a_known_debt_reports_known_not_fail(self):
        r = row(10**7, 16 * 10**7, known='tickets.md#x')
        self.assertEqual(r['status'], 'KNOWN')
        self.assertEqual(s.overall([r]), 'PASS')

    def test_a_known_probe_within_budget_says_its_override_can_go(self):
        r = row(10**7, 4 * 10**7, known='tickets.md#x')
        self.assertEqual(r['status'], 'PASS')
        self.assertIn('override can go', r['reason'])

    def test_a_probe_budget_overrides_the_default(self):
        self.assertEqual(row(10**7, 2 * 10**7, budget=0.25)['status'], 'FAIL')

    def test_no_counter_and_tiny_bodies_are_refused(self):
        self.assertIn('no instruction counter', row(None, 10**7)['reason'])
        self.assertIn('under', row(10, 40)['reason'])
        self.assertEqual(row(10, 40)['status'], 'FAIL')

    def test_stack_spread_fails_even_a_known_probe(self):
        probe = {**PROBE, 'stack': True, 'known': 'ticket'}
        flat = s.verdict(probe, {'instructions': 10**7}, {'instructions': 4 * 10**7, 'stack': [8 << 20] * 5})
        deep = s.verdict(probe, {'instructions': 10**7}, {'instructions': 4 * 10**7,
                                                          'stack': [8 << 20, 6 << 20, 4 << 20]})
        self.assertEqual((flat['status'], flat['stack_growth']), ('PASS', 0))
        self.assertEqual(deep['status'], 'FAIL')
        self.assertIn('native stack', deep['reason'])

    def test_overall_needs_rows(self):
        self.assertEqual(s.overall([]), 'FAIL')


class Parsing(unittest.TestCase):
    def test_scaling_lines_and_probe_entries(self):
        text = 'noise\nSCALING {"name":"a","n":3,"k":4,"budget":1.25,"known":null,"gui":false,"steps":1,"stack":true}\n'
        (raw,) = s.scaling_lines(text)
        probe = s.probe_entry(raw)
        self.assertEqual((probe['name'], probe['n'], probe['known'], probe['stack'], probe['gui']),
                         ('a', 3, None, True, False))

    def test_step_paints_counts_each_steps_submitted_paints_before_the_next_mark(self):
        text = '\n'.join(['{"diagnostic":"paint-submitted","presentation":1}', '{"scaling":"start"}',
                          '{"scaling":"step","i":0}', 'noise', '{"diagnostic":"paint-submitted","presentation":2}',
                          '{"diagnostic":"paint","presentation":1}',
                          '{"scaling":"step","i":1}', '{"diagnostic":"paint","presentation":2}',
                          '{"scaling":"step","i":2}', '{"diagnostic":"paint-submitted","presentation":3}',
                          '{"scaling":"end"}', '{"diagnostic":"paint-submitted","presentation":4}'])
        self.assertEqual(s.step_paints(text), [1, 0, 1])

    def test_table_names_status_and_reason(self):
        text = s.table([row(10**7, 16 * 10**7, known='tickets.md#x')])
        self.assertIn('KNOWN', text)
        self.assertIn('known: tickets.md#x', text)


if __name__ == '__main__':
    unittest.main()
