#!/usr/bin/env python3
"""Exercise priority/FIFO admission, legacy lock cooperation and hard timeout."""
import fcntl
import json
import multiprocessing
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import romacs_accept_queue as queue


def enqueue(root, label, rank):
    queue.run_ticket(root, label, rank, 10,
                     [sys.executable, '-c', 'from pathlib import Path; import sys; p=Path(sys.argv[1]); f=p.open("a"); f.write(sys.argv[2]+"\\n"); f.close()', str(root / 'order'), label])


class QueueTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_priority_fifo_and_legacy_lock(self):
        with (self.root / '.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            jobs = []
            for label, rank in [('low', 50), ('first', 10), ('second', 10)]:
                process = multiprocessing.Process(target=enqueue, args=(self.root, label, rank))
                process.start()
                jobs.append(process)
                deadline = time.monotonic() + 10
                while len(queue.tickets(self.root)) < len(jobs):
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(.01)
            self.assertFalse((self.root / 'order').exists())
            self.assertEqual([t['label'] for t in queue.tickets(self.root)], ['first', 'second', 'low'])
            fcntl.flock(lock, fcntl.LOCK_UN)
        for process in jobs:
            process.join(10)
            self.assertEqual(process.exitcode, 0)
        self.assertEqual((self.root / 'order').read_text().splitlines(), ['first', 'second', 'low'])
        self.assertEqual(queue.tickets(self.root), [])

    def test_legacy_lock_descriptor_reaches_mode_owner_child(self):
        code = ('import os,sys,json; from pathlib import Path; '
                'fd=int(os.environ["ROMACS_BENCH_LOCK_FD"]); '
                'p=Path(sys.argv[1]); '
                'p.write_text(json.dumps({"inode":os.fstat(fd).st_ino}))')
        destination = self.root / 'descriptor.json'
        self.assertEqual(queue.run_ticket(self.root, 'fd', 10, 10,
                         [sys.executable, '-c', code, str(destination)]), 0)
        self.assertEqual(json.loads(destination.read_text())['inode'], (self.root / '.lock').stat().st_ino)

    def test_timeout_kills_child_and_releases_ticket(self):
        with patch.object(queue, 'MAX_HOLD', .1), self.assertRaises(subprocess.TimeoutExpired):
            queue.run_ticket(self.root, 'slow', 10, .05,
                             [sys.executable, '-c', 'import time; time.sleep(60)'])
        self.assertEqual(queue.tickets(self.root), [])
        with (self.root / '.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_queue_transport_error_has_infrastructure_exit(self):
        with patch.object(queue.subprocess, 'run', return_value=subprocess.CompletedProcess([], 255)) as run:
            self.assertEqual(queue.list_remote('benchmark.example.org'), 2)
        self.assertEqual(run.call_args.args[0][:2], ['ssh', 'benchmark.example.org'])

    def test_queue_remote_requires_explicit_destination(self):
        with patch.object(queue.subprocess, 'run') as run:
            with self.assertRaises(TypeError):
                queue.list_remote()
        run.assert_not_called()

    def test_invalid_budget_no_effect(self):
        for duration in (0, -1, 2701):
            with self.assertRaises(ValueError):
                queue.run_ticket(self.root, 'bad', 10, duration, ['false'])
        self.assertFalse((self.root / 'queue').exists())

    def test_dead_ticket_reaped(self):
        directory = self.root / 'queue'
        directory.mkdir()
        (directory / 'dead.json').write_text(json.dumps({'pid': 999999999, 'priority': 0, 'sequence': 0}))
        self.assertEqual(queue.tickets(self.root), [])
        self.assertEqual(list(directory.iterdir()), [])

    def test_roadmap_priority(self):
        lanes = ['mem-m1', 'w22', 'sqlite', 'evp1', 'w17-s2', 'w4m', 'probe']
        self.assertEqual(sorted(lanes, key=queue.priority), lanes)

if __name__ == '__main__':
    unittest.main()
