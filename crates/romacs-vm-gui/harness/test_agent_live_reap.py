#!/usr/bin/env python3
"""agent_live's teardown: no process a run starts outlives it, however
detached (a supervisor's setsid), without a compositor or binary."""
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import unittest

import agent_live as al


def detached(mark, where):
    """A process started as a supervisor is, in a session of its own (its
    starter gone at once), MARK in its environment; its pid, which it
    writes in WHERE."""
    env = {**os.environ, al.MARK: mark}
    subprocess.run(['setsid', '--fork', 'sh', '-c', f'echo $$ > {where}.tmp && mv {where}.tmp {where}; exec sleep 60'],
                   env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                   timeout=10, check=True)
    al.until(lambda: Path(where).exists(), 10, 'detached child started')
    return int(Path(where).read_text())


class Reap(unittest.TestCase):
    def setUp(self):
        self.base = Path(tempfile.mkdtemp(prefix='agent-live-reap-')).resolve()
        self.run = self.base / 'server-hk-x'
        self.other = Path(str(self.base) + '-other')

    def tearDown(self):
        al.end_marked(str(self.base))
        al.end_marked(str(self.other))
        shutil.rmtree(self.base)

    def test_a_detached_child_of_a_run_is_found_and_ended(self):
        pid = detached(str(self.run / 'home'), self.base / 'pid')
        self.assertIn(pid, al.marked_processes(str(self.base)))
        self.assertIn(pid, al.end_marked(str(self.run)))
        self.assertEqual(al.marked_processes(str(self.base)), [])

    def test_another_run_s_process_is_left(self):
        # A sibling whose path only starts with the same characters.
        pid = detached(str(self.other), self.base / 'pid')
        self.assertEqual(al.end_marked(str(self.base)), [])
        self.assertEqual(al.marked_processes(str(self.other)), [pid])


if __name__ == '__main__':
    unittest.main()
