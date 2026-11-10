#!/usr/bin/env python3
"""perkey_live's validation, without a compositor or binary."""
import unittest

import perkey_live as pk


class Counters(unittest.TestCase):
    def test_full_time_count_is_read(self):
        self.assertEqual(pk.perf_count('# started\n4449412383,,instructions:u,10403168678,100.00,,\n'), 4449412383)

    def test_uncounted_unsupported_and_partial_counts_are_refused(self):
        for line in ('<not counted>,,instructions:u,0,0.00,,', '<not supported>,,instructions:u,0,0.00,,',
                     '12345,,instructions:u,5000,50.00,,', 'no counter here'):
            with self.assertRaises(pk.Invalid):
                pk.perf_count(line)

    def test_an_idle_thread_that_never_ran_counts_zero_only_when_idle(self):
        line = '<not counted>,,instructions:u,0,100.00,,'
        self.assertEqual(pk.perf_count(line, idle=True), 0)
        with self.assertRaises(pk.Invalid):
            pk.perf_count('<not counted>,,instructions:u,5000,100.00,,', idle=True)


class Expectations(unittest.TestCase):
    def test_points_follow_insert_goal_column_and_forward_motion(self):
        text = 'a' * 40 + pk.text_of(200)
        self.assertEqual(pk.expected_point('self-insert', text, 0, 40), 40)
        line40 = text.split('\n')[40]
        start = sum(len(line) + 1 for line in text.split('\n')[:40])
        self.assertEqual(pk.expected_point('C-n', text, 40, 40), start + min(40, len(line40)))
        self.assertEqual(pk.expected_point('C-f', text, 100, 40), 140)

    def test_next_line_clamps_to_a_shorter_line(self):
        self.assertEqual(pk.next_line('abcdef\nab\n', 5, 5), 9)

    def test_motion_past_the_text_is_refused(self):
        with self.assertRaises(pk.Invalid):
            pk.expected_point('C-n', pk.text_of(10), 0, 40)
        with self.assertRaises(pk.Invalid):
            pk.expected_point('C-f', 'abc', 0, 40)


class Delivery(unittest.TestCase):
    def reply(self, command='forward-char', signal=None):
        return {'native_command': command, 'signal': signal, 'presentation': 3}

    def test_every_key_must_run_its_command_without_a_signal(self):
        self.assertEqual(pk.delivery([self.reply()] * 3, 'forward-char', 3), [])
        self.assertTrue(pk.delivery([self.reply()] * 2, 'forward-char', 3))
        self.assertTrue(pk.delivery([self.reply()] * 2 + [self.reply('next-line')], 'forward-char', 3))
        self.assertTrue(pk.delivery([self.reply(signal={'condition': 'end-of-buffer'})] * 3, 'forward-char', 3))

    def test_a_refused_phase_leaves_the_session_and_its_other_phases(self):
        phase = lambda n, problems=(): {'instructions': n, 'seconds': 10, 'problems': list(problems), 'paints': 8}
        result = {'valid': True, 'problems': [], 'idle': [(0, 10), (0, 10)],
                  'phases': {'self-insert': phase(400), 'C-n': phase(800, ['point 3, expected 4']), 'C-f': phase(40)}}
        self.assertIsNone(pk.invalid(result))
        summary = pk.per_key([result], 4)
        self.assertEqual(summary['self-insert']['valid']['median'], 100)
        self.assertIsNone(summary['C-n']['valid'])
        self.assertEqual(summary['C-n']['refused'], {'per_key': pk.spread([200]), 'why': ['point 3, expected 4']})

    def test_a_failed_save_or_disagreeing_idles_reject_the_session(self):
        self.assertTrue(pk.invalid({'valid': True, 'problems': ['1 command replies for 1 keys'], 'phases': {}}))
        self.assertTrue(pk.invalid({'valid': True, 'problems': [], 'phases': {}, 'idle': [(1e6, 10), (5e7, 10)]}))


class Latency(unittest.TestCase):
    def sample(self, sequence, rendered=5_000_000, unchanged=False):
        return {'diagnostic': 'latency-key', 'key_sequence': sequence,
                'key_to_render_observed_ns': None if unchanged or rendered is None else rendered,
                **({'unchanged': True} if unchanged else {})}

    def test_one_sample_per_key_each_rendered_or_unchanged(self):
        samples = [self.sample(7), self.sample(8, unchanged=True), self.sample(9)]
        self.assertEqual(pk.latency_check(samples, 3), ([5_000_000, 5_000_000], 1, []))

    def test_one_sample_cannot_stand_for_the_other_keys(self):
        self.assertTrue(pk.latency_check([self.sample(7)], 3)[2])

    def test_a_key_with_no_render_time_is_refused_not_dropped(self):
        rendered, _, problems = pk.latency_check([self.sample(1), self.sample(2, rendered=None)], 2)
        self.assertEqual(rendered, [5_000_000])
        self.assertTrue(problems)

    def test_samples_must_be_distinct_keys_of_one_run(self):
        self.assertTrue(pk.latency_check([self.sample(1), self.sample(1)], 2)[2])
        self.assertTrue(pk.latency_check([self.sample(1), self.sample(3)], 2)[2])
        self.assertTrue(pk.latency_check([self.sample(None), self.sample(2)], 2)[2])


class Summaries(unittest.TestCase):
    def test_idle_phases_must_agree_unless_both_negligible(self):
        self.assertEqual(pk.idle_rate([(100_000, 10), (300_000, 10)]), 20_000)
        with self.assertRaises(pk.Invalid):
            pk.idle_rate([(1_000_000, 10), (50_000_000, 10)])

    def test_spread_is_flagged_over_ten_percent(self):
        self.assertFalse(pk.spread([100, 102, 105])['flag'])
        summary = pk.spread([100, 100, 120])
        self.assertTrue(summary['flag'])
        self.assertAlmostEqual(summary['spread'], .2)

    def test_per_key_subtracts_the_idle_rate(self):
        phase = lambda n: {'instructions': n, 'seconds': 10, 'problems': [], 'paints': 4}
        result = {'idle': [(1_000, 10), (1_000, 10)],
                  'phases': {'self-insert': phase(4_001_000), 'C-n': phase(8_001_000), 'C-f': phase(1_000)}}
        summary = pk.per_key([result], 4)
        self.assertEqual(summary['self-insert']['valid']['median'], 1_000_000)
        self.assertEqual(summary['C-f']['valid']['median'], 0)
        self.assertEqual(summary['C-f']['paints_per_key']['median'], 1)

    def test_text_point_reads_the_first_window_of_the_last_publication(self):
        published = [{'diagnostic': 'cursor-publication', 'points': [{'id': 1, 'point_byte': n}, {'id': 2, 'point_byte': 0}]}
                     for n in (0, 7)]
        self.assertEqual(pk.text_point(published), 7)


class HostCalls(unittest.TestCase):
    ROWS = ('(find-file top 1 ((layout.measure local 5) (read-file cross 2)))\n'
            '(next-line top 1 ((layout.measure local 9) (read-file cross 2)))\n'
            '(next-line top 1 ((sender.input cross 1) (layout.measure local 12) (read-file cross 2)))\n'
            '(next-line top 1 ((sender.input cross 1) (layout.measure local 13) (read-file cross 2)))\n'
            '(scroll-up-command top 1 ((sender.input cross 1) (layout.measure local 20) (read-file cross 2)))\n'
            '(scroll-up-command top 40 ((sender.input cross 1) (layout.measure local 30) (read-file cross 2)))\n'
            '(scroll-up-command top 40 ((sender.input cross 1) (layout.measure local 31) (read-file cross 2)))\n'
            '(self-insert-command minibuffer 1 ((sender.input cross 3) (layout.measure local 32) (read-file cross 2)))\n'
            '(self-insert-command minibuffer 1 ((sender.input cross 5) (layout.measure local 32) (read-file cross 2)))\n'
            '(auto-fill-mode top 40 ((sender.input cross 9) (layout.measure local 34) (read-file cross 2)))\n')

    def rows(self):
        return pk.host_call_rows(pk.sexps(self.ROWS))

    def test_a_command_counts_only_after_the_same_command_in_the_same_context(self):
        self.assertEqual(pk.host_call_segments(self.rows()), [
            ('next-line', {'sender.input': ('cross', 1), 'layout.measure': ('local', 3)}),
            ('next-line', {'layout.measure': ('local', 1)}),
            ('scroll-up-command', {'layout.measure': ('local', 10)}),
            ('minibuffer self-insert-command', {'sender.input': ('cross', 2)})])

    def test_each_side_and_name_is_averaged_over_the_commands(self):
        report = pk.host_calls_per_command(pk.host_call_segments(self.rows()))
        self.assertEqual(report['next-line'], {
            'commands': 2, 'local': {'mean': 2, 'max': 3}, 'cross': {'mean': .5, 'max': 1},
            'by_name': {'layout.measure': {'side': 'local', 'mean': 2}, 'sender.input': {'side': 'cross', 'mean': .5}}})
        self.assertEqual(report['scroll-up-command']['commands'], 1)

    def test_a_prompt_counts_from_the_command_before_it_to_the_one_that_ends_it(self):
        prompts = pk.host_call_prompts(self.rows())
        self.assertEqual(prompts, {'minibuffer': {'sender.input': ('cross', 8), 'layout.measure': ('local', 3)}})
        self.assertEqual(pk.host_calls_per_prompt_key([prompts, prompts], {'minibuffer': 4}), {'minibuffer': {
            'prompts': 2, 'keys_each': 4, 'local': .75, 'cross': 2,
            'by_name': {'layout.measure': {'side': 'local', 'per_key': .75}, 'sender.input': {'side': 'cross', 'per_key': 2}}}})

    def test_rows_count_after_the_setup(self):
        rows = [('self-insert-command', 'minibuffer', '1', {}), ('eval-expression', 'top', '1', {}), ('next-line', 'top', '1', {})]
        self.assertEqual(pk.after_setup(rows), rows[2:])
        self.assertEqual(pk.after_setup(rows[2:]), rows[2:])

    def test_the_search_letters_occur_three_times(self):
        self.assertEqual(pk.search_word('a' * 40 + pk.text_of(5)), 'ordina')
        with self.assertRaises(pk.Invalid):
            pk.search_word('aaaaaaaaaaaaaaaaaaaaaaaa one two three')


if __name__ == '__main__':
    unittest.main()
