"""scaling_live.py's pure parts: exponents, verdicts, overrides and parsing."""
import math
from pathlib import Path
import sys
import tempfile
import unittest

import scaling_live as s

PROBE = {'name': 'p', 'n': 100, 'k': 4, 'budget': 1.25, 'known': None, 'gui': False, 'steps': 1, 'stack': False,
         'idle': False}


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

    def test_idle_steps_set_aside_for_a_paint_are_on_the_row(self):
        probe = {**PROBE, 'gui': True, 'idle': True}
        r = s.verdict(probe, {'instructions': 10**7, 'disturbed': 1}, {'instructions': 10**7, 'disturbed': 0})
        self.assertEqual((r['status'], r['disturbed']), ('PASS', 1))
        self.assertIn('1 idle steps set aside for a paint', s.table([r]))
        self.assertNotIn('disturbed', row(10**7, 4 * 10**7))
        self.assertIn('no instruction counter', s.verdict(probe, {'instructions': None, 'disturbed': 0}, {'instructions': 10**7})['reason'])

    def test_overall_needs_rows(self):
        self.assertEqual(s.overall([]), 'FAIL')


class Parsing(unittest.TestCase):
    def test_scaling_lines_and_probe_entries(self):
        text = 'noise\nSCALING {"name":"a","n":3,"k":4,"budget":1.25,"known":null,"gui":false,"steps":1,"stack":true}\n'
        (raw,) = s.scaling_lines(text)
        probe = s.probe_entry(raw)
        self.assertEqual((probe['name'], probe['n'], probe['known'], probe['stack'], probe['gui'], probe['idle']),
                         ('a', 3, None, True, False, False))

    def test_every_step_paints_and_a_tenth_of_idle_steps_may(self):
        painting, idle = {**PROBE, 'steps': 2}, {**PROBE, 'steps': 10, 'idle': True}
        self.assertIsNone(s.paints_problem(painting, [1, 2]))
        self.assertIn('[1, 0]', s.paints_problem(painting, [1, 0]))
        self.assertIn('[1]', s.paints_problem(painting, [1]))
        self.assertIsNone(s.paints_problem(idle, [0] * 10))
        self.assertIsNone(s.paints_problem(idle, [0] * 9 + [1]))
        self.assertIn('10 idle steps: [0, 0, 0, 0, 0, 0, 0, 0, 2, 1]', s.paints_problem(idle, [0] * 8 + [2, 1]))
        self.assertIn('idle steps: [0]', s.paints_problem(idle, [0]))

    def test_idle_work_is_the_unpainted_steps_scaled_to_all(self):
        self.assertEqual(s.idle_work([100, 100, 100, 100], [0, 0, 0, 0]), (400, 0))
        # The painted step's 900 is no idle work: the three others stand for four.
        self.assertEqual(s.idle_work([100, 900, 100, 100], [0, 1, 0, 0]), (400, 1))
        self.assertEqual(s.idle_work([100, None], [0, 0]), (None, 0))
        self.assertEqual(s.idle_work([100, 100], [1, 1]), (None, 2))

    def test_step_paints_counts_each_steps_submitted_paints_before_the_next_mark(self):
        text = '\n'.join(['{"diagnostic":"paint-submitted","presentation":1}', '{"scaling":"start"}',
                          '{"scaling":"step","i":0}', 'noise', '{"diagnostic":"paint-submitted","presentation":2}',
                          '{"diagnostic":"paint","presentation":1}',
                          '{"scaling":"step","i":1}', '{"diagnostic":"paint","presentation":2}',
                          '{"scaling":"step","i":2}', '{"diagnostic":"paint-submitted","presentation":3}',
                          '{"scaling":"end"}', '{"diagnostic":"paint-submitted","presentation":4}'])
        self.assertEqual(s.step_paints(text), [1, 0, 1])

    def test_a_slope_probe_is_held_by_its_slope_not_its_exponent(self):
        # 10 M of fixed work and 2 instructions per unit of N a step, 4 steps
        # at N 100 and 400: the exponent says it grows, the slope that it is
        # within 3 a unit.
        flat = row(10**7 + 2 * 100 * 4, 10**7 + 2 * 400 * 4, slope=3.0, steps=4, budget=0.0)
        self.assertEqual((flat['status'], flat['slope'], flat['slope_budget']), ('PASS', 2.0, 3.0))
        steep = row(10**7, 10**7 + 5 * 300 * 4, slope=3.0, steps=4)
        self.assertEqual(steep['status'], 'FAIL')
        self.assertIn('slope 5.0 over budget 3.0', steep['reason'])
        self.assertEqual(row(10**7, 10**7 + 5 * 300 * 4, slope=3.0, steps=4, known='tickets.md#y')['status'], 'KNOWN')
        self.assertNotIn('slope', row(10**7, 4 * 10**7))
        text = s.table([flat])
        self.assertIn('| slope 3.0 | PASS', text)
        self.assertIn('slope 2.0 a unit of N a step', text)
        entry = s.probe_entry({'name': 'a', 'n': 3, 'slope': 2})
        self.assertEqual((entry['slope'], s.probe_entry({'name': 'a', 'n': 3})['slope']), (2.0, None))

    def test_table_names_status_and_reason(self):
        text = s.table([row(10**7, 16 * 10**7, known='tickets.md#x')])
        self.assertIn('KNOWN', text)
        self.assertIn('known: tickets.md#x', text)



class Bounded(unittest.TestCase):
    """A session is bounded by CPU, which load does not spend; wall time only
    ends one that waits without working."""
    def test_a_session_that_ends_returns_its_output(self):
        done = s.bounded('quick', [sys.executable, '-c', 'print("SCALING {}")'], 5)
        self.assertEqual(done.stdout.strip(), 'SCALING {}')

    def test_a_session_that_keeps_computing_is_refused_by_its_cpu_bound(self):
        with self.assertRaisesRegex(s.Refused, r'spin: used its 1s of CPU'):
            s.bounded('spin', [sys.executable, '-c', 'while True: pass'], 1)

    def test_a_session_past_its_hard_limit_says_so(self):
        ignore = 'import signal; signal.signal(signal.SIGXCPU, signal.SIG_IGN)\nwhile True: pass'
        with self.assertRaisesRegex(s.Refused, r'ignore: killed \(SIGKILL\) under a CPU limit of 1s, hard 2s'):
            s.bounded('ignore', [sys.executable, '-c', ignore], 1)

    def test_a_session_that_waits_without_working_is_refused_by_the_wall_clock(self):
        with self.assertRaisesRegex(s.Refused, r'sleep: no end after 2s, within 1s of CPU: hung'):
            s.bounded('sleep', [sys.executable, '-c', 'import time; time.sleep(60)'], 0.2)

class World(unittest.TestCase):
    """A measured session runs from its HOME's batch world: one that fell
    back to source, which a session tells only with its diagnostics on, is
    refused rather than measured."""
    def stub(self, directory):
        binary = Path(directory) / 'romacs'
        binary.write_text('#!/bin/sh\n'
                          'if [ "$ROMACS_DIAGNOSTICS" = 1 ]; then\n'
                          '  echo \'romacs-world: {"mode":"source","reason":"stale"}\' >&2\n'
                          'fi\n'
                          'echo \'SCALING {"instructions": 1}\'\n')
        binary.chmod(0o755)
        return binary

    def test_a_measured_session_from_source_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            binary, home = self.stub(directory), Path(directory)
            with self.assertRaisesRegex(s.Refused, r'ran from source, not its world'):
                s.batch(binary, home, '(scaling-run (quote p) 1)', 5, world=True)

    def test_an_unmeasured_session_may_run_from_source(self):
        with tempfile.TemporaryDirectory() as directory:
            binary, home = self.stub(directory), Path(directory)
            self.assertEqual(s.batch(binary, home, '(scaling-list)', 5), [{'instructions': 1}])


if __name__ == '__main__':
    unittest.main()
