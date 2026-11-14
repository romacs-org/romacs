"""display_stages_live.py's pure parts: steps, figures, refusals, bars and the table."""
import unittest

import display_stages_live as d

NAMES = ('step-instructions', 'collections', 'cycles', 'instructions', 'conses', 'windows-rebuilt', 'band-expansions', 'layouts',
         'frames-submitted', 'lines-laid', 'rows-wrapped')
IDLE = {'name': 'idle', 'paints': False}
PAINTS = {'name': 'typed', 'paints': True}


def readings(*rows):
    """Readings whose consecutive differences are ROWS, each NAMES' counts."""
    out, total = [dict.fromkeys(NAMES, 0)], dict.fromkeys(NAMES, 0)
    for row in rows:
        row = (*row, *[0] * (len(NAMES) - len(row)))
        total = {name: total[name] + count for name, count in zip(NAMES, row)}
        out.append(total)
    return out


# A step of 900 instructions, 500 of them in one redisplay that built one
# window, expanded two bands, submitted a frame and laid one line out, one
# row of it wrapped.
STEP = (900, 0, 1, 500, 7, 1, 2, 0, 1, 1, 1)
QUIET = (300, 0, 1, 100, 0, 0, 2, 0, 0)


class Figures(unittest.TestCase):
    def test_a_step_is_the_difference_of_two_readings(self):
        (first, second) = d.steps(readings(STEP, QUIET))
        self.assertEqual((first['step-instructions'], first['instructions'], second['frames-submitted']), (900, 500, 0))

    def test_figures_are_medians_and_the_instructions_90th_percentiles(self):
        heavy = (5000, 1, 1, 4000, 9, 1, 2, 3, 1)
        f = d.figures(d.steps(readings(*[STEP] * 7, *[heavy] * 3)))
        self.assertEqual((f['step_instructions'], f['display_instructions'], f['display_conses'], f['layouts']), (900, 500, 7, 0))
        self.assertEqual((f['display_instructions_p90'], 'step_instructions_p90' in f), (4000, False))
        # The 90th percentile leaves out one step in twenty that stands out, not one in ten.
        calm = d.figures(d.steps(readings(*[STEP] * 9, heavy)))
        self.assertEqual((calm['display_instructions_p90'], d.quantile([1, 2, 3, 4], .5), d.quantile([1], .9)), (4000, 3, 1))
        self.assertEqual(d.figures(d.steps(readings(*[STEP] * 19, heavy)))['display_instructions_p90'], 500)

    def test_a_count_only_some_scenarios_keep_is_a_figure_where_kept(self):
        counted = [dict(r, **{'fontified-chars': 40 * i}) for i, r in enumerate(readings(STEP, STEP, STEP))]
        self.assertEqual(d.figures(d.steps(counted))['chars_fontified'], 40)
        self.assertNotIn('chars_fontified', d.figures(d.steps(readings(STEP, STEP))))


class Refusals(unittest.TestCase):
    def test_a_whole_measurement_is_not_refused(self):
        self.assertIsNone(d.refusal(PAINTS, {'readings': readings(STEP, STEP)}, 2))
        self.assertIsNone(d.refusal(IDLE, {'readings': readings(QUIET, QUIET)}, 2))

    def test_missing_steps_and_counters_are_refused(self):
        self.assertIn('2 readings for 2 steps', d.refusal(PAINTS, {'readings': readings(STEP)}, 2))
        blind = readings(STEP)
        blind[1]['instructions'] = None
        self.assertIn('no instruction counter', d.refusal(PAINTS, {'readings': blind}, 1))

    def test_a_step_without_a_redisplay_is_refused(self):
        self.assertIn('steps [1] did not run one redisplay', d.refusal(IDLE, {'readings': readings(QUIET, (300, 0, 0, 0, 0, 0, 0, 0, 0))}, 2))

    def test_a_step_of_two_redisplays_is_odd(self):
        twice = (600, 0, 2, 200, 0, 0, 4, 0, 0)
        self.assertEqual(d.odd_steps(IDLE, d.steps(readings(QUIET, twice, QUIET))), [1])

    def test_a_scenario_paints_every_step_or_none(self):
        self.assertIn('steps [1] did not run one redisplay that submitted a frame', d.refusal(PAINTS, {'readings': readings(STEP, QUIET)}, 2))
        self.assertIn('steps [1] did not run one redisplay that submitted no frame', d.refusal(IDLE, {'readings': readings(QUIET, STEP)}, 2))

    def test_a_tenth_of_the_steps_may_be_odd(self):
        disturbed = {'readings': readings(*[QUIET] * 9, STEP)}
        self.assertIsNone(d.refusal(IDLE, disturbed, 10))
        # The figures are the rule-keeping steps': the painted one is in none.
        row = d.verdict(IDLE, disturbed, 10, {})
        self.assertEqual((row['odd'], row['figures']['frames_submitted'], row['figures']['display_instructions_p90']), ([9], 0, 100))
        self.assertEqual(len(row['steps']), 10)
        self.assertIn('steps [8, 9]', d.refusal(IDLE, {'readings': readings(*[QUIET] * 8, STEP, STEP)}, 10))


class Bars(unittest.TestCase):
    def setUp(self):
        self.result = {'readings': readings(STEP, STEP), 'chars': 10}
        self.bars = d.bars_of([d.verdict(PAINTS, self.result, 2, {})])['typed']

    def test_written_bars_pass_the_run_they_came_from(self):
        row = d.verdict(PAINTS, self.result, 2, {'typed': self.bars})
        self.assertEqual((row['status'], row['off']), ('PASS', []))
        self.assertEqual(d.overall([row]), 'PASS')

    def test_counts_are_held_as_measured_and_instructions_and_conses_with_a_margin(self):
        self.assertEqual((self.bars['windows_rebuilt'], self.bars['band_expansions'], self.bars['frames_submitted']), (1, 2, 1))
        self.assertEqual((self.bars['display_conses'], d.ceiling('display_conses', 7439), d.ceiling('layouts', 2)), (8, 8190, 2))
        self.assertEqual((self.bars['display_instructions'], self.bars['display_instructions_p90']), (550, 550))
        self.assertEqual((d.ceiling('step_instructions', 5_234_567), d.ceiling('step_instructions', 0)), (5_760_000, 0))
        self.assertEqual((d.ceiling('display_instructions', 900), d.ceiling('display_instructions', 999)), (990, 1100))

    def test_a_figure_over_its_bar_fails_by_name(self):
        row = d.verdict(PAINTS, self.result, 2, {'typed': {**self.bars, 'windows_rebuilt': 0, 'display_instructions': 499}})
        self.assertEqual(row['status'], 'FAIL')
        self.assertEqual(row['reason'], 'display_instructions 500, over 499; windows_rebuilt 1, over 0')

    def test_a_count_under_its_bar_fails_and_cheaper_instructions_pass(self):
        # A step that rebuilds no window where its bar holds one no longer
        # does its change, or got cheaper: either way the bar is to move.
        row = d.verdict(PAINTS, self.result, 2, {'typed': {**self.bars, 'windows_rebuilt': 2}})
        self.assertEqual((row['status'], row['reason']), ('FAIL', 'windows_rebuilt 1, under 2'))
        cheaper = {**self.bars, 'display_instructions': 10**6, 'display_conses': 10**6, 'step_instructions': 10**6}
        self.assertEqual(d.verdict(PAINTS, self.result, 2, {'typed': cheaper})['status'], 'PASS')

    def test_a_scenario_or_figure_without_a_bar_fails(self):
        self.assertIn('no bars', d.verdict(PAINTS, self.result, 2, {})['reason'])
        partial = {k: v for k, v in self.bars.items() if k != 'layouts'}
        self.assertEqual(d.verdict(PAINTS, self.result, 2, {'typed': partial})['reason'], 'layouts 0, no bar')

    def test_a_refused_measurement_fails_and_has_no_figures(self):
        row = d.verdict(PAINTS, {'readings': readings(STEP, QUIET)}, 2, {'typed': self.bars})
        self.assertEqual(row['status'], 'FAIL')
        self.assertNotIn('figures', row)
        self.assertEqual((d.overall([row]), d.overall([]), d.bars_of([row])), ('FAIL', 'FAIL', {}))

    def test_the_table_shows_figures_or_the_reason(self):
        text = d.table([d.verdict(PAINTS, self.result, 2, {'typed': self.bars}), d.verdict(IDLE, {'readings': []}, 2, {})])
        self.assertIn('| typed | 0.00M | 0.00M | 0.00M | 7 | 1 | 2 | 0 | 1 | 1 | 1 | PASS |', text)
        self.assertIn('| idle | - | - | - | - | - | - | - | - | - | - | FAIL — 0 readings for 2 steps |', text)


if __name__ == '__main__':
    unittest.main()
