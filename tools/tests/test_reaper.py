"""The reaper (tools/lib/reaper.py) a test file's host runs under."""
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
REAPER = ROOT / 'tools/lib/reaper.py'
SLEEPER = '''import ctypes, os, pathlib, sys, time
if len(sys.argv) > 2:
    ctypes.CDLL(None).prctl(15, sys.argv[2].encode('latin-1'), 0, 0, 0)
pid = pathlib.Path(sys.argv[1])
pid.with_suffix('.new').write_text(str(os.getpid()))
pid.with_suffix('.new').rename(pid)
time.sleep(60)
'''


def alive(pid):
    state = Path(f'/proc/{pid}/stat')
    try:
        return state.read_bytes().rsplit(b')', 1)[1].split()[0] != b'Z'
    except (OSError, IndexError):
        return False


def wait_for_file(path, seconds=20):
    deadline = time.monotonic() + seconds
    while not path.exists():
        if time.monotonic() > deadline:
            raise AssertionError(f'{path} never came')
        time.sleep(0.01)
    return int(path.read_text())


class Reaper(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.home = Path(temp.name)
        self.addCleanup(self.kill_recorded)

    def kill_recorded(self):
        for pidfile in self.home.glob('*.pid'):
            try:
                os.kill(int(pidfile.read_text()), signal.SIGKILL)
            except (ProcessLookupError, ValueError):
                pass

    def reap(self, *command, **options):
        return subprocess.run([sys.executable, '-I', str(REAPER), *command], capture_output=True,
                              text=True, timeout=60, **options)

    def test_a_process_with_a_name_that_is_no_text_is_ended_too(self):
        # A name the kernel cut mid-character is not UTF-8.
        name = ('xx' + 'α' * 7).encode()[:15].decode('latin-1')
        pidfile = self.home / 'named.pid'
        host = (f'import subprocess, sys, pathlib, time\n'
                f'subprocess.Popen([sys.executable, "-c", {SLEEPER!r}, {str(pidfile)!r}, {name!r}], start_new_session=True)\n'
                f'while not pathlib.Path({str(pidfile)!r}).exists(): time.sleep(0.01)\n')
        result = self.reap(sys.executable, '-c', host)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(alive(int(pidfile.read_text())))

    def test_a_host_killed_outright_is_said_so(self):
        result = self.reap('sh', '-c', 'kill -KILL $$')
        self.assertEqual(result.returncode, -signal.SIGKILL, result.stderr)
        self.assertNotIn('Traceback', result.stderr)
        result = self.reap('sh', '-c', 'kill -TERM $$')
        self.assertEqual(result.returncode, -signal.SIGTERM, result.stderr)

    def test_the_host_starts_with_no_signal_ignored(self):
        result = self.reap('grep', 'SigIgn', '/proc/self/status')
        mask = int(result.stdout.split()[1], 16)
        for number in (signal.SIGPIPE, signal.SIGXFSZ):
            self.assertFalse(mask & (1 << (number - 1)), f'{signal.Signals(number).name} ignored')

    def test_a_killed_reaper_takes_its_host(self):
        pidfile = self.home / 'host.pid'
        reaper = subprocess.Popen([sys.executable, '-I', str(REAPER), sys.executable, '-c', SLEEPER, str(pidfile)])
        host = wait_for_file(pidfile)
        reaper.kill()
        reaper.wait()
        deadline = time.monotonic() + 5
        while alive(host) and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertFalse(alive(host), 'the host outlived its reaper')

    def test_an_ignored_sigchld_is_not_inherited(self):
        result = self.reap('true', preexec_fn=lambda: signal.signal(signal.SIGCHLD, signal.SIG_IGN))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_nothing_runs_once_its_caller_is_gone(self):
        gone = subprocess.Popen(['true'])
        gone.wait()
        mark = self.home / 'ran'
        result = self.reap('touch', str(mark), env=dict(os.environ, REAPER_PARENT=str(gone.pid)))
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(mark.exists(), 'the command ran with its caller gone')
        result = self.reap('touch', str(mark), env=dict(os.environ, REAPER_PARENT=str(os.getpid())))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(mark.exists())


if __name__ == '__main__':
    unittest.main()
