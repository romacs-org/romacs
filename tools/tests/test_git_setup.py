#!/usr/bin/env python3
"""tools/git-setup, on a fixture repository: the retired merge drivers it
removes, and the hook that keeps the trunk to the merge queue."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]


def run(root, *args, check=True, env=None):
    result = subprocess.run(args, cwd=root, env=env, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True)
    if check and result.returncode:
        raise RuntimeError(f'{args!r}: {result.returncode}\n{result.stdout}')
    return result


def git(root, *args, **kwargs):
    return run(root, 'git', *args, **kwargs)


class GitSetupTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='git-setup-test-', dir=os.environ.get('TMPDIR'))
        self.root = Path(self.tmp.name) / 'repo'
        self.root.mkdir()
        git(self.root, 'init', '-q')
        (self.root / 'tools').mkdir()
        shutil.copy2(ROOT / 'tools/git-setup', self.root / 'tools/git-setup')
        shutil.copytree(ROOT / 'tools/hooks', self.root / 'tools/hooks')
        for key, value in (('user.name', 'Setup fixture'), ('user.email', 'setup@example.invalid'),
                           ('commit.gpgsign', 'false')):
            git(self.root, 'config', '--local', key, value)
        git(self.root, 'add', '.')
        git(self.root, 'commit', '-qm', 'Fixture base')

    def tearDown(self):
        self.tmp.cleanup()

    def plain_session(self, *args, land=None):
        """git ARGS as a session outside the merge queue: the checks run under the
        queue's own process tree, which the hook lets through, so the command is
        started orphaned (setsid -f), with ROMACS_LAND only as LAND says."""
        env = {k: v for k, v in os.environ.items() if k != 'ROMACS_LAND'}
        if land is not None:
            env['ROMACS_LAND'] = land
        out = Path(self.tmp.name) / 'plain'
        status, go = out.with_suffix('.status'), out.with_suffix('.go')
        status.unlink(missing_ok=True)
        go.unlink(missing_ok=True)
        # The child waits for GO, written once setsid has exited and so the child is reparented.
        script = 'until [ -e "$OUT.go" ]; do sleep 0.01; done; git "$@" > "$OUT" 2>&1; echo $? > "$OUT.status"'
        subprocess.run(['setsid', '-f', 'sh', '-c', script, 'sh', *args],
                       cwd=self.root, env={**env, 'OUT': str(out)}, check=True)
        go.touch()
        for _ in range(300):
            if status.exists() and status.read_text().endswith('\n'):
                return int(status.read_text()), out.read_text()
            time.sleep(0.05)
        self.fail(f'git {args} never finished')

    def test_setup_removes_the_retired_merge_drivers(self):
        retired = (('merge.romacs-derived.driver', 'python3 tools/derived driver %A %B'),
                   ('alias.merge-derived', '!python3 tools/derived merge'),
                   ('merge.romacs-floors.driver', 'python3 -B tools/lib/smells.py merge %O %A %B'),
                   ('merge.romacs-unsafe-floors.driver', 'python3 -B tools/lib/unsafe_census.py merge %O %A %B'))
        for key, value in retired:
            git(self.root, 'config', '--local', key, value)
        run(self.root, sys.executable, 'tools/git-setup')
        for key, _ in retired:
            self.assertNotEqual(git(self.root, 'config', '--local', key, check=False).returncode, 0, key)
        run(self.root, sys.executable, 'tools/git-setup')                 # once more: nothing left to remove

    def test_setup_installs_the_hook_that_keeps_the_trunk_to_the_queue(self):
        run(self.root, sys.executable, 'tools/git-setup', 'setup')
        code, said = self.plain_session('update-ref', 'refs/heads/beta', 'HEAD')
        self.assertNotEqual(code, 0)
        self.assertIn('moves only through the merge queue', said)
        self.assertEqual(self.plain_session('update-ref', 'refs/heads/lane', 'HEAD')[0], 0)  # any other branch moves
        self.assertNotEqual(self.plain_session('update-ref', 'refs/heads/beta', 'HEAD', land='0')[0], 0)
        named = self.plain_session('-c', 'review.note=x pack-refs', 'update-ref', 'refs/heads/beta', 'HEAD')
        self.assertNotEqual(named[0], 0, 'a value naming pack-refs is not the command')
        self.assertEqual(self.plain_session('update-ref', 'refs/heads/beta', 'HEAD', land='1')[0], 0)
        self.assertEqual(self.plain_session('pack-refs', '--all')[0], 0)  # storage only: beta is loose here
        self.assertEqual(self.plain_session('gc', '-q')[0], 0)
        self.assertEqual(git(self.root, 'rev-parse', 'beta').stdout, git(self.root, 'rev-parse', 'HEAD').stdout)
        queue = Path(self.tmp.name) / 'romacs-merge'                    # the queue, by its script's name
        queue.write_text("import subprocess\nsubprocess.run(['git', 'update-ref', '-d', 'refs/heads/beta'], check=True)\n")
        run(self.root, sys.executable, str(queue))
        self.assertNotEqual(git(self.root, 'rev-parse', '--verify', '-q', 'beta', check=False).returncode, 0)


if __name__ == '__main__':
    unittest.main()
