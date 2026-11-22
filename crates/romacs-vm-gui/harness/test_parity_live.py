"""Focused harness tests; no Cargo build or ambient compositor required."""
import os
import tempfile
import unittest
from pathlib import Path

import layout_journey_live
import parity_live


class KeyTests(unittest.TestCase):
    def test_a_character_is_typed_under_its_modifier(self):
        self.assertEqual(parity_live.C('n'), ['-M', 'ctrl', 'n', '-m', 'ctrl'])
        self.assertEqual(parity_live.M('>'), ['-M', 'alt', '>', '-m', 'alt'])

    def test_a_named_key_is_pressed_not_spelled(self):
        # Typed, `space' would be C-s C-p C-a C-c C-e.
        self.assertEqual(parity_live.C('space'), ['-M', 'ctrl', '-k', 'space', '-m', 'ctrl'])
        self.assertEqual(parity_live.M('Return'), ['-M', 'alt', '-k', 'Return', '-m', 'alt'])

    def test_no_journey_key_spells_a_name(self):
        for group in layout_journey_live.DOWN + layout_journey_live.KEYS:
            if '-M' in group:
                key = group[2:-2]
                self.assertTrue(len(key[-1]) == 1 or key[0] == '-k', group)

    def test_press_counts_each_key_once(self):
        # Engine.press waits for the session's input count to pass these:
        # one event per character or pressed keysym, none for a modifier.
        # (Measured in pinned GNU and Romacs for every journey key.)
        counts = [parity_live.inputs(g) for g in (parity_live.C('n'), parity_live.M('>'), parity_live.T('the'),
                                                  parity_live.K('Return'), parity_live.C('space'))]
        self.assertEqual(counts, [1, 1, 3, 1, 1])
        for group in layout_journey_live.DOWN + layout_journey_live.KEYS:
            self.assertGreater(parity_live.inputs(group), 0, group)


class PrivateTreeTests(unittest.TestCase):
    def test_home_and_work_are_not_memory_only(self):
        # A file under the runtime directory is a memory-only file to the
        # editor (lisp/session/romacs-ram-files.el): a restart leaves its
        # buffer out.  A journey's files must be ordinary ones.
        with tempfile.TemporaryDirectory() as base:
            root, home, env = parity_live.private_tree('romacs', Path(base), '/nonexistent')
            runtime = Path(env['XDG_RUNTIME_DIR'])
            self.assertTrue(runtime.is_dir())
            self.assertEqual(runtime.stat().st_mode & 0o777, 0o700)
            for path in (home, root / 'work', root / 'probe', Path(env['TMPDIR'])):
                self.assertFalse(path.is_relative_to(runtime), path)

    def test_a_socket_path_too_long_is_refused_before_the_compositor(self):
        with tempfile.TemporaryDirectory() as base:
            # One byte over the limit when TMPDIR leaves room for it, more
            # when TMPDIR is long already (a test runner's private one).
            pad = max(1, parity_live.SOCKET_PATH_MAX - len(os.fsencode(base)) - len('/wayland-0'))
            root = Path(base) / ('r' * pad)
            root.mkdir()
            size = len(os.fsencode(str(root / 'wayland-0')))
            self.assertGreater(size, parity_live.SOCKET_PATH_MAX)
            engine = parity_live.Engine('romacs', [], root, None, {}, 1)
            with self.assertRaisesRegex(RuntimeError, f'socket path {size} > '):
                engine.spawn('/nonexistent')
            self.assertEqual(engine.processes, [])


class StartupFailedTests(unittest.TestCase):
    def test_a_stopped_startup_is_found_in_whole_lines_read_once(self):
        with tempfile.TemporaryDirectory() as root:
            engine = parity_live.Engine('romacs', [], Path(root), None, {}, 1)
            log = Path(root) / 'engine.log'
            log.write_bytes(b'romacs-world: {}\nromacs: evaluator st')
            self.assertIsNone(engine.startup_failed(), 'a line not yet whole')
            with log.open('ab') as more:
                more.write(b'opped: x.el: No such file\nromacs-exit: {}\n')
            self.assertEqual(engine.startup_failed(), 'romacs: evaluator stopped: x.el: No such file')
            self.assertEqual(engine.log_read, log.stat().st_size)
            log.write_bytes(b'')
            self.assertEqual(engine.startup_failed(), 'romacs: evaluator stopped: x.el: No such file', 'kept once found')

    def test_a_launcher_s_session_log_is_read_once_it_is_made(self):
        with tempfile.TemporaryDirectory() as root:
            engine = parity_live.Engine('romacs', [], Path(root), None, {}, 1)
            engine.editor_log = 'home/sessions/*/stderr.log'
            self.assertIsNone(engine.startup_failed(), 'no session yet')
            session = Path(root) / 'home/sessions/s1'
            session.mkdir(parents=True)
            (session / 'stderr.log').write_bytes(b'romacs: evaluator stopped: y.el: gone\n')
            self.assertEqual(engine.startup_failed(), 'romacs: evaluator stopped: y.el: gone')


if __name__ == '__main__':
    unittest.main()
