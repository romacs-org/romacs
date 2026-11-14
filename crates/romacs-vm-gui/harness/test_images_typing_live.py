#!/usr/bin/env python3
"""images_typing_live's validation, without a compositor or binary."""
import unittest

import images_typing_live as itl


class Sizes(unittest.TestCase):
    def test_every_image_must_have_a_size(self):
        self.assertEqual(itl.size_problems('images-typing-sizes ((600 . 400) (600 . 400))', 2), [])
        self.assertTrue(itl.size_problems('images-typing-sizes ((600 . 400))', 2))
        self.assertTrue(itl.size_problems(None, 1))

    def test_a_failed_load_is_refused_by_its_placeholder_size(self):
        problems = itl.size_problems('images-typing-sizes ((600 . 400) (30 . 30))', 2)
        self.assertEqual(problems, ["image 2 failed to load (the placeholder's size)"])


class Landing(unittest.TestCase):
    STAT = '4242 (image-decode) {state} 1 1 1 0 -1 4194368 10 0 0 0 {utime} {stime} 0 0 20 0 1 0'

    def test_a_thread_reads_as_its_state_and_cpu_ticks(self):
        self.assertEqual(itl.thread_state(self.STAT.format(state='R', utime=120, stime=3)), ('R', 123))
        self.assertEqual(itl.thread_state('7 (a (b) c) S 1 1 1 0 -1 0 0 0 0 0 5 6 0'), ('S', 11))

    def test_threads_wait_only_asleep_at_both_readings_with_no_cpu_between(self):
        self.assertTrue(itl.waiting([('S', 10)], [('S', 10)]))
        self.assertTrue(itl.waiting([], []), 'no worker: nothing to wait for')
        self.assertFalse(itl.waiting([('S', 10)], [('S', 11)]), 'it ran between')
        self.assertFalse(itl.waiting([('R', 10)], [('S', 10)]), 'it was running')
        self.assertFalse(itl.waiting([('D', 10)], [('D', 10)]), 'it was reading its file')
        self.assertFalse(itl.waiting([], [('S', 0)]), 'a worker started between')

    def test_a_frame_in_flight_holds_the_span_open(self):
        submitted = lambda n: {'diagnostic': 'paint-submitted', 'presentation': n}
        painted = lambda n: {'diagnostic': 'paint', 'presentation': n}
        # A decode landed while frame 7 was in flight: the evaluator
        # submitted 8 and waits for it to paint.
        self.assertFalse(itl.presented([submitted(7), painted(6), submitted(8)]))
        self.assertFalse(itl.presented([submitted(7), submitted(8), painted(7)]))
        self.assertTrue(itl.presented([submitted(7), submitted(8), painted(8)]))
        self.assertTrue(itl.presented([submitted(7), submitted(8), painted(9)]), 'a later paint retires it')
        self.assertTrue(itl.presented([]))


if __name__ == '__main__':
    unittest.main()
