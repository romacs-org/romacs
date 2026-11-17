"""render_stages_live.py's pure parts: frames, figures, refusals, bars and the table."""
import json
import unittest

import render_stages_live as r

TYPED = {'name': 'typed'}


def frame(scenes=1, result='Submitted', **counts):
    """A `frame-cost' line: a scene frame of 46 rows tessellated, of
    900 instructions in its draw, 400 of them the driver's, and 100 in its
    scene's ingestion, unless COUNTS say otherwise."""
    line = {'diagnostic': 'frame-cost', 'presentation': 1, 'work': 'rebuild', 'result': result, 'scenes': scenes,
            'ingest_instructions': 100, 'frame_instructions': 900, 'acquire_instructions': 50, 'submit_instructions': 300,
            'present_instructions': 50, 'rows_tessellated': 46, 'rows_reused': 0, 'cursor_rows': 1,
            'glyphs_walked': 3000, 'font_identities': 6000, 'bytes_uploaded': 800_000}
    return {**line, **counts}


def stderr(*lines, start=True):
    """A session's stderr: other diagnostics, the start mark, then LINES."""
    head = [{'diagnostic': 'paint', 'presentation': 0}, frame(rows_tessellated=1)] + ([{'stages': 'start'}] if start else [])
    return 'a line of text\n' + '\n'.join(json.dumps(line) for line in head + list(lines)) + '\n{not json\n'


class Frames(unittest.TestCase):
    def test_frames_are_the_cost_lines_after_the_start_mark_that_reached_the_surface(self):
        drawn = r.frames(stderr(frame(), {'diagnostic': 'paint'}, frame(result='Occluded'), frame(scenes=0), {'stages': 'end'}, frame(rows_tessellated=47)))
        self.assertEqual([(f['rows_tessellated'], f['scenes']) for f in drawn], [(46, 1), (46, 0), (47, 1)])
        self.assertEqual(r.frames(stderr(frame(), start=False)), [])

    def test_a_session_without_a_start_mark_after_a_good_one_has_no_frames(self):
        earlier = stderr(*[frame()] * 30).encode()
        markerless = 'the session died early\n'.encode()
        self.assertEqual(r.frames(r.since(earlier + markerless, len(earlier))), [])
        self.assertEqual(len(r.frames(r.since(earlier, 0))), 30)

    def test_an_earlier_session_in_the_same_stderr_is_not_counted(self):
        both = stderr(*[frame()] * 30) + stderr(frame(rows_tessellated=47))
        self.assertEqual([f['rows_tessellated'] for f in r.frames(both)], [47])

    def test_own_instructions_are_the_ingestion_and_the_draw_less_the_drivers_part(self):
        self.assertEqual((r.own(frame()), r.driver(frame())), (600, 400))


class Figures(unittest.TestCase):
    def test_figures_are_medians_over_scene_frames_and_extras_are_counted_apart(self):
        heavy = frame(rows_tessellated=90, frame_instructions=5000)
        extra = frame(scenes=0, ingest_instructions=0, frame_instructions=700)
        held, said = r.figures([frame()] * 3 + [heavy] * 2 + [extra] * 2, 5)
        self.assertEqual((held['rows_tessellated'], held['cursor_rows'], held['own_instructions'], held['bytes_uploaded']), (46, 1, 600, 800_000))
        self.assertEqual((held['extra_frames'], said['scene_frames'], said['driver_instructions'], said['extra_own_instructions']), (4, 5, 400, 300))
        self.assertEqual(r.figures([frame()], 1)[1]['extra_own_instructions'], 0)

    def test_extra_frames_are_per_ten_steps_rounded_up(self):
        extra = frame(scenes=0)
        self.assertEqual([r.figures([frame()] * 30 + [extra] * n, 30)[0]['extra_frames'] for n in (0, 1, 3, 30)], [0, 1, 1, 10])


class Refusals(unittest.TestCase):
    def test_nine_tenths_of_the_steps_drew_a_scene_frame(self):
        self.assertIsNone(r.refusal([frame()] * 9, 10))
        self.assertEqual(r.refusal([frame()] * 8 + [frame(scenes=0)] * 5, 10), '8 scene frames for 10 steps')

    def test_a_thread_without_a_counter_is_refused(self):
        blind = frame(frame_instructions=0, ingest_instructions=0)
        self.assertEqual(r.refusal([blind] * 2, 2), 'no instruction counter on the render thread')


class Bars(unittest.TestCase):
    def setUp(self):
        self.text = stderr(frame(), frame())
        self.bars = r.bars_of([r.verdict(TYPED, self.text, 2, {})])['typed']

    def test_written_bars_pass_the_run_they_came_from(self):
        row = r.verdict(TYPED, self.text, 2, {'typed': self.bars})
        self.assertEqual((row['status'], row['off'], len(row['frames'])), ('PASS', [], 2))

    def test_rows_are_held_as_counted_and_costs_with_a_margin(self):
        self.assertEqual((self.bars['rows_tessellated'], self.bars['rows_reused'], self.bars['cursor_rows']), (46, 0, 1))
        self.assertEqual((self.bars['own_instructions'], self.bars['glyphs_walked'], self.bars['extra_frames']), (660, 3300, 0))
        self.assertEqual((r.ceiling('own_instructions', 5_234_567), r.ceiling('rows_tessellated', 46), r.ceiling('bytes_uploaded', 0)), (5_760_000, 46, 0))

    def test_a_row_count_off_its_bar_fails_either_way_and_a_cheaper_cost_passes(self):
        more = r.verdict(TYPED, self.text, 2, {'typed': {**self.bars, 'rows_tessellated': 1}})
        self.assertEqual((more['status'], more['reason']), ('FAIL', 'rows_tessellated 46, over 1'))
        fewer = r.verdict(TYPED, self.text, 2, {'typed': {**self.bars, 'rows_reused': 45}})
        self.assertEqual(fewer['reason'], 'rows_reused 0, under 45')
        cheaper = {**self.bars, 'own_instructions': 10**6, 'glyphs_walked': 10**6, 'extra_frames': 5}
        self.assertEqual(r.verdict(TYPED, self.text, 2, {'typed': cheaper})['status'], 'PASS')

    def test_a_cost_over_its_bar_fails_by_name(self):
        row = r.verdict(TYPED, self.text, 2, {'typed': {**self.bars, 'own_instructions': 599}})
        self.assertEqual(row['reason'], 'own_instructions 600, over 599')

    def test_a_scenario_or_figure_without_a_bar_fails(self):
        self.assertIn('no bars', r.verdict(TYPED, self.text, 2, {})['reason'])
        partial = {k: v for k, v in self.bars.items() if k != 'font_identities'}
        self.assertEqual(r.verdict(TYPED, self.text, 2, {'typed': partial})['reason'], 'font_identities 6000, no bar')

    def test_a_refused_measurement_fails_and_has_no_figures(self):
        row = r.verdict(TYPED, stderr(frame()), 2, {'typed': self.bars})
        self.assertEqual((row['status'], row['reason'], 'figures' in row), ('FAIL', '1 scene frames for 2 steps', False))
        self.assertEqual(r.bars_of([row]), {})

    def test_the_table_shows_figures_or_the_reason(self):
        text = r.table([r.verdict(TYPED, self.text, 2, {'typed': self.bars}), r.verdict({'name': 'gone'}, '', 2, {})])
        self.assertIn('| typed | 46 | 0 | 1 | 3000 | 6000 | 781K | 0.00M | 0.00M | 0.0/step | 0.00M | PASS |', text)
        self.assertIn('| gone | - | - | - | - | - | - | - | - | - | - | FAIL — 0 scene frames for 2 steps |', text)


if __name__ == '__main__':
    unittest.main()
