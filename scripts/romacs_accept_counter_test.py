#!/usr/bin/env python3
"""Real perf_event_open proof: count worker threads, exclude control children."""
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

import romacs_accept_counter as counter


class CounterTests(unittest.TestCase):
    def test_event_flags(self):
        attr = counter.attributes()
        self.assertEqual(struct.unpack_from('I', attr, 4)[0], 128)
        flags = struct.unpack_from('Q', attr, 40)[0]
        self.assertTrue(flags & (1 << 1))
        self.assertTrue(flags & (1 << 35))
        self.assertTrue(flags & (1 << 5))

    def measure(self, mode):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ctl, ack, out = root / 'control', root / 'ack', root / 'result'
            os.mkfifo(ctl)
            os.mkfifo(ack)
            code = '''import os,sys,threading
c=os.open(sys.argv[1],os.O_RDWR); a=os.open(sys.argv[2],os.O_RDWR)
def control(s):
 os.write(c,(s+'\\n').encode()); assert os.read(a,4)==b'ack\\n'
def work():
 x=0
 for i in range(100000): x+=i
control('enable')
if sys.argv[3]=='thread':
 t=threading.Thread(target=work); t.start(); t.join()
elif sys.argv[3]=='fork':
 p=os.fork()
 if p==0: work(); os._exit(0)
 os.waitpid(p,0)
control('disable')
'''
            result = subprocess.run([sys.executable, str(Path(counter.__file__)), str(out), str(ctl), str(ack),
                                     sys.executable, '-c', code, str(ctl), str(ack), mode],
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            return int(out.read_text().split(';')[0])

    def test_real_thread_inheritance_without_fork_inheritance(self):
        thread_count = self.measure('thread')
        fork_count = self.measure('fork')
        self.assertGreater(thread_count, fork_count * 3, (thread_count, fork_count))


if __name__ == '__main__':
    unittest.main()
