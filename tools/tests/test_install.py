"""The curl installer against a local release: verification, replacement,
removal, and the refusals, under a private HOME."""
import hashlib
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'install.sh'
NAME = 'romacs-x86_64-linux'
# Stands in for the binary: records its arguments and succeeds.
STUB = ('#!/bin/sh\nprintf "%s\\n" "$*" >>"$HOME/romacs-calls"\n'
        'case "$1" in --compile-lisp-tree) exit "$(cat "$HOME/status" 2>/dev/null || echo 0)" ;; esac\n')


def release(root, damaged=False):
    """Write a release under ROOT/latest/download and return ROOT."""
    tree = root / 'tree' / NAME
    (tree / 'bin').mkdir(parents=True)
    (tree / 'bin/romacs').write_text(STUB)
    (tree / 'bin/romacs').chmod(0o755)
    for path in ('share/applications/romacs.desktop', 'share/icons/hicolor/scalable/apps/romacs.svg'):
        (tree / path).parent.mkdir(parents=True)
        (tree / path).write_text('Exec=romacs %F\n')
    out = root / 'latest/download'
    out.mkdir(parents=True)
    archive = out / f'{NAME}.tar.gz'
    with tarfile.open(archive, 'w:gz') as tar:
        tar.add(tree, arcname=NAME)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (out / 'SHA256SUMS').write_text(f'{digest}  {NAME}.tar.gz\n')
    if damaged:
        archive.write_bytes(archive.read_bytes()[:64])
    return root


class InstallTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.home = self.root / 'my home'
        self.home.mkdir()
        self.prefix = self.home / '.local/opt' / NAME

    def run_script(self, *args, releases=None, path=None):
        environment = {'HOME': str(self.home), 'PATH': path or os.environ['PATH'], 'TMPDIR': str(self.root),
                       'ROMACS_RELEASES': f'file://{releases or self.good}'}
        return subprocess.run(['bash', '-s', '--', *args], input=SCRIPT.read_bytes(), env=environment,
                              capture_output=True)

    @property
    def good(self):
        path = self.root / 'good'
        return path if path.exists() else release(path)

    def test_installs_links_and_prepares_the_runtime(self):
        run = self.run_script('--no-compile')
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual((self.home / '.local/bin/romacs').resolve(), self.prefix / 'bin/romacs')
        self.assertTrue((self.home / '.local/share/applications/romacs.desktop').exists())
        self.assertEqual((self.home / 'romacs-calls').read_text(),
                         f'-Q --loadup --world-build\n--world-store {self.prefix}/bin/romacs-vm-gui.worlds --world-build --batch\n')
        entry = (self.home / '.local/share/applications/romacs.desktop').read_text()
        self.assertEqual(entry, f'Exec="{self.home}/.local/bin/romacs" %F\n')

    def test_compiling_runs_last_and_status_2_is_success(self):
        (self.home / 'status').write_text('2\n')
        self.assertEqual(self.run_script().returncode, 0)
        calls = (self.home / 'romacs-calls').read_text().splitlines()
        self.assertEqual(calls[0], '-Q --loadup --world-build')
        self.assertEqual(calls[1], f'--world-store {self.prefix}/bin/romacs-vm-gui.worlds --world-build --batch')
        self.assertRegex(calls[2], r'^--compile-lisp-tree --jobs \d+$')

    def test_a_reinstall_leaves_nothing_of_the_old_tree(self):
        self.run_script('--no-compile')
        (self.prefix / 'stale').write_text('old\n')
        self.assertEqual(self.run_script('--no-compile').returncode, 0)
        self.assertFalse((self.prefix / 'stale').exists())
        self.assertEqual(sorted(p.name for p in self.prefix.parent.iterdir()), [NAME])

    def test_a_damaged_download_changes_nothing(self):
        self.run_script('--no-compile')
        (self.prefix / 'kept').write_text('still here\n')
        run = self.run_script('--no-compile', releases=release(self.root / 'bad', damaged=True))
        self.assertEqual(run.returncode, 1)
        self.assertIn(b'nothing was installed', run.stderr)
        self.assertTrue((self.prefix / 'kept').exists())

    def test_a_failed_placement_restores_the_old_tree(self):
        self.run_script('--no-compile')
        (self.prefix / 'kept').write_text('still here\n')
        stubs = self.root / 'stubs'
        stubs.mkdir()
        (stubs / 'mv').write_text(f'#!/bin/sh\ncase "$1" in *.romacs-new*/{NAME}) exit 1 ;; esac\n'
                                  'exec /usr/bin/mv "$@"\n')
        (stubs / 'mv').chmod(0o755)
        run = self.run_script('--no-compile', path=f'{stubs}:{os.environ["PATH"]}')
        self.assertEqual(run.returncode, 1, run.stderr)
        self.assertTrue((self.prefix / 'kept').exists())
        self.assertEqual(sorted(p.name for p in self.prefix.parent.iterdir()), [NAME])

    def test_a_foreign_launcher_is_left_alone(self):
        launcher = self.home / '.local/bin/romacs'
        launcher.parent.mkdir(parents=True)
        launcher.write_text('mine\n')
        run = self.run_script('--no-compile')
        self.assertEqual(run.returncode, 1)
        self.assertIn(b'already exists', run.stderr)
        self.assertEqual(self.run_script('--uninstall').returncode, 0)
        self.assertEqual(launcher.read_text(), 'mine\n')

    def test_uninstall_removes_only_what_it_placed(self):
        self.run_script('--no-compile')
        init = self.home / '.config/romacs/init.el'
        init.parent.mkdir(parents=True)
        init.write_text(';; mine\n')
        self.assertEqual(self.run_script('--uninstall').returncode, 0)
        self.assertFalse(self.prefix.exists())
        self.assertFalse((self.home / '.local/bin/romacs').is_symlink())
        self.assertFalse((self.home / '.local/share/applications/romacs.desktop').exists())
        self.assertTrue(init.exists())

    def test_an_unknown_argument_refuses_before_any_work(self):
        run = self.run_script('--nope')
        self.assertEqual(run.returncode, 1)
        self.assertFalse((self.home / '.local').exists())


if __name__ == '__main__':
    unittest.main()
