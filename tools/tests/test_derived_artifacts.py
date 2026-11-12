#!/usr/bin/env python3
"""The derived-file frontdoor and its source-first merge protocol, on a fixture.

The one tracked derived file left is the bundled autoloads. The fixture's
generator derives it from `autoload-input-*.txt`, so a merge of two branches
must regenerate it to describe both.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/lib'))
from derived_artifacts import ARTIFACTS  # noqa: E402

AUTOLOADS = ARTIFACTS[0]
MERGED = 'merged artifacts must describe both source branches'
OWNED = ('tools/check-derived', 'tools/derived', 'tools/lib/derived_artifacts.py', '.gitattributes',
         'lisp/bundled.json')
AUTOLOADS_FIXTURE = '''import json
from pathlib import Path
output = Path(json.loads(Path('lisp/bundled.json').read_text())['autoloads'])
output.parent.mkdir(parents=True, exist_ok=True)
inputs = sorted(p.stem for p in Path('.').glob('autoload-input-*.txt'))
output.write_text(f';;; {output.name} --- fixture\\n' + ''.join(f';; {i}\\n' for i in inputs)
                  + f';;; {output.name} ends here\\n')
'''
# check-derived's other checks are not under test here: each passes.
STUB = 'GNU_SOURCE = "."\nif __name__ == "__main__":\n    raise SystemExit(0)\n'
CHECKERS = ('tools/lib/python_tests.py', 'tools/lib/lisp_oracle.py', 'test/gnu/run.py', 'tools/lib/parity_ratchet.py',
            'tools/lib/gnu_arity.py', 'tools/lib/gnu_defvars.py', 'tools/lib/icon_font.py', 'tools/lib/bundled_fonts.py',
            'tools/lib/treesit_grammars.py', 'tools/lib/unsafe_census.py', 'tools/lib/smells.py',
            'tools/lib/divergence_index.py', 'tools/lib/lisp_tree.py')


def run(root, *args, check=True, env=None):
    result = subprocess.run(args, cwd=root, env=env, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True)
    if check and result.returncode:
        raise RuntimeError(f'{args!r}: {result.returncode}\n{result.stdout}')
    return result


def git(root, *args, **kwargs):
    return run(root, 'git', *args, **kwargs)


def fixture(root):
    root.mkdir()
    git(root, 'init', '-q')
    for name in OWNED:
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, root / name)
    for name in CHECKERS:
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_text(STUB)
    (root / 'tools/lib/bundled_autoloads.py').write_text(AUTOLOADS_FIXTURE)
    (root / 'autoload-input-base.txt').write_text('base\n')
    for key, value in (('user.name', 'Derived fixture'), ('user.email', 'derived@example.invalid'),
                       ('commit.gpgsign', 'false'), ('core.hooksPath', '/dev/null')):
        git(root, 'config', '--local', key, value)
    regenerate(root)
    git(root, 'add', '.')
    git(root, 'commit', '-qm', 'Fixture base')


def regenerate(root):
    return run(root, sys.executable, 'tools/derived', 'regenerate')


def check_derived(root):
    return run(root, sys.executable, 'tools/check-derived', check=False)


class DerivedTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='derived-test-', dir=os.environ.get('TMPDIR'))
        self.root = Path(self.tmp.name) / 'repo'
        fixture(self.root)
        self.path = self.root / AUTOLOADS

    def tearDown(self):
        self.tmp.cleanup()

    def test_setup_installs_the_hook_that_keeps_the_trunk_to_the_queue(self):
        shutil.copytree(ROOT / 'tools/hooks', self.root / 'tools/hooks')
        git(self.root, 'config', '--local', '--unset', 'core.hooksPath')
        run(self.root, sys.executable, 'tools/derived', 'setup')
        moved = git(self.root, 'update-ref', 'refs/heads/beta', 'HEAD', check=False)
        self.assertNotEqual(moved.returncode, 0)
        self.assertIn('moves only through the merge queue', moved.stdout)
        git(self.root, 'update-ref', 'refs/heads/lane', 'HEAD')          # any other branch moves
        git(self.root, 'update-ref', 'refs/heads/beta', 'HEAD', env={**os.environ, 'ROMACS_LAND': '1'})
        queue = Path(self.tmp.name) / 'romacs-merge'                    # the queue, by its script's name
        queue.write_text("import subprocess\nsubprocess.run(['git', 'update-ref', '-d', 'refs/heads/beta'], check=True)\n")
        run(self.root, sys.executable, str(queue))
        self.assertNotEqual(git(self.root, 'rev-parse', '--verify', '-q', 'beta', check=False).returncode, 0)

    def test_a_fresh_tree_passes(self):
        result = check_derived(self.root)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn('check-derived: ok', result.stdout)

    def test_corruption_is_refused_before_healing(self):
        good = self.path.read_bytes()
        for corrupt, why in ((good[:-5], 'not a complete generated Lisp file'),
                             (b'<<<<<<< ours\n' + good, 'unresolved conflict marker')):
            with self.subTest(why=why):
                self.path.write_bytes(corrupt)
                result = check_derived(self.root)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(why, result.stdout)
                self.assertEqual(self.path.read_bytes(), corrupt, 'validation must not heal corruption')
        self.path.write_bytes(good)

    def test_explicit_regeneration_recovers_corruption(self):
        good = self.path.read_bytes()
        self.path.write_text('<<<<<<< corrupt\n')
        regenerate(self.root)
        self.assertEqual(self.path.read_bytes(), good)

    def test_a_stale_artifact_is_regenerated_and_refused(self):
        (self.root / 'autoload-input-new.txt').write_text('new\n')
        result = check_derived(self.root)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('stale derived artifacts', result.stdout)
        self.assertIn(';; autoload-input-new', self.path.read_text())

    def test_a_failed_generator_is_refused(self):
        (self.root / 'tools/lib/bundled_autoloads.py').write_text('raise SystemExit(37)\n')
        result = check_derived(self.root)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('returned non-zero exit status 37', result.stdout)

    def test_generator_output_with_a_marker_is_refused(self):
        (self.root / 'tools/lib/bundled_autoloads.py').write_text(
            AUTOLOADS_FIXTURE.replace("+ f';;; {output.name} ends here", "+ '=======\\n' + f';;; {output.name} ends here"))
        result = check_derived(self.root)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('unresolved conflict marker', result.stdout)

    def test_sources_first_merge(self):
        prepare_branches(self.root)
        git(self.root, 'merge-derived', 'derived-right')
        assert_merged(self.root)
        before = self.path.read_bytes()
        regenerate(self.root)
        self.assertEqual(before, self.path.read_bytes(), 'merge result must equal canonical regeneration')

    def test_native_merge_fails_closed_then_finish(self):
        prepare_branches(self.root)
        result = git(self.root, 'merge', '--no-ff', 'derived-right', check=False)
        self.assertNotEqual(result.returncode, 0, 'native merge must defer, not accept arbitrary artifact bytes')
        self.assertIn(AUTOLOADS, git(self.root, 'diff', '--name-only', '--diff-filter=U').stdout)
        run(self.root, sys.executable, 'tools/derived', 'finish')
        assert_merged(self.root)

    def test_finish_refuses_unstaged_or_untracked_sources(self):
        prepare_branches(self.root)
        git(self.root, 'merge', '--no-ff', 'derived-right', check=False)
        source = self.root / 'autoload-input-base.txt'
        for untracked in (False, True):
            with self.subTest(untracked=untracked):
                if untracked:
                    added = self.root / 'tools' / 'extra.py'
                    added.write_text('# never committed\n')
                else:
                    source.write_text('edited\n')
                result = run(self.root, sys.executable, 'tools/derived', 'finish', check=False)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('stage all source resolutions', result.stdout)
                if untracked:
                    added.unlink()
                else:
                    source.write_text('base\n')
        run(self.root, sys.executable, 'tools/derived', 'finish')
        assert_merged(self.root)

    def test_a_source_conflict_is_neither_resolved_nor_committed(self):
        prepare_branches(self.root, source_conflict=True)
        before = git(self.root, 'rev-parse', 'HEAD').stdout
        result = run(self.root, sys.executable, 'tools/derived', 'merge', 'derived-right', check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('resolve and stage source conflicts first', result.stdout)
        self.assertEqual(before, git(self.root, 'rev-parse', 'HEAD').stdout)
        self.assertIn('shared-source.txt', git(self.root, 'diff', '--name-only', '--diff-filter=U').stdout)


def prepare_branches(root, source_conflict=False):
    run(root, sys.executable, 'tools/derived', 'setup')
    if source_conflict:
        (root / 'shared-source.txt').write_text('base\n')
        git(root, 'add', '.')
        git(root, 'commit', '-qm', 'Conflict base')
    base = git(root, 'rev-parse', 'HEAD').stdout.strip()
    for branch in ('left', 'right'):
        git(root, 'checkout', '-qb', 'derived-' + branch, base)
        (root / f'autoload-input-{branch}.txt').write_text(branch + '\n')
        if source_conflict:
            (root / 'shared-source.txt').write_text(branch + '\n')
        regenerate(root)
        git(root, 'add', '.')
        git(root, 'commit', '-qm', 'Source changes on ' + branch)
    git(root, 'checkout', '-q', 'derived-left')


def assert_merged(root):
    text = (root / AUTOLOADS).read_text()
    assert ';; autoload-input-left' in text and ';; autoload-input-right' in text, MERGED
    assert not git(root, 'status', '--porcelain').stdout, 'merge result must be committed and clean'
    assert len(git(root, 'show', '-s', '--format=%P').stdout.split()) == 2, 'must create a genuine merge commit'


if __name__ == '__main__':
    unittest.main()
