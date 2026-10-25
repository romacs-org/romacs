#!/usr/bin/env python3
"""Derived frontdoor regressions and isolated source-first merge protocol tests.

Default tests use real Python index generators and a small snapshot-generator
fixture. --real additionally runs the production Rust generator in a disposable
clone containing the current candidate, using the prepared project environment.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
from derived_artifacts import ARTIFACTS

MERGED = 'merged artifacts must describe both source branches'
OWNED = ('scripts/check-derived', 'scripts/derived', 'tools/derived_artifacts.py',
         'tools/primitive_index.py', 'tools/divergence_index.py', '.gitattributes')
SNAPSHOT_FIXTURE = '''#!/usr/bin/env python3
import json
from pathlib import Path
root = Path.cwd()
json.loads((root / 'crates/romacs-introspect/data/registry.json').read_bytes())
values = sorted(p.stem for p in root.glob('snapshot-input-*.txt'))
for name in ['crates/romacs-introspect-live/data/registry.json', 'crates/romacs-introspect/data/registry.json']:
    (root / name).parent.mkdir(parents=True, exist_ok=True)
    (root / name).write_text(json.dumps({'fixture_sources': values}) + '\\n')
'''


def run(root, *args, check=True, env=None):
    result = subprocess.run(args, cwd=root, env=env, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True)
    if check and result.returncode:
        raise RuntimeError(f'{args!r}: {result.returncode}\n{result.stdout}')
    return result


def git(root, *args, **kwargs):
    return run(root, 'git', *args, **kwargs)


def copy(root, name):
    (root / name).parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / name, root / name)


def fixture(root, real=False):
    if real:
        run(root.parent, 'git', 'clone', '--quiet', '--shared', '--no-hardlinks', str(ROOT), str(root))
        patch = git(ROOT, 'diff', '--binary', 'HEAD').stdout
        subprocess.run(['git', 'apply', '--binary'], cwd=root, input=patch, text=True, check=True)
    else:
        root.mkdir()
        git(root, 'init', '-q')
        for name in ('lib.rs', 'prim.rs'):
            copy(root, 'crates/romacs-vm/src/' + name)
        for path in (ROOT / 'crates/romacs-vm/src/prims').glob('*.rs'):
            copy(root, path.relative_to(ROOT).as_posix())
        copy(root, 'DIVERGENCES.md')
        for name in ('tools/lisp_oracle.py', 'tools/parity_ratchet.py'):
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text('raise SystemExit(0)\n')
    for name in OWNED:
        copy(root, name)
    git(root, 'config', '--local', 'user.name', 'Derived fixture')
    git(root, 'config', '--local', 'user.email', 'derived@example.invalid')
    git(root, 'config', '--local', 'commit.gpgsign', 'false')
    git(root, 'config', '--local', 'core.hooksPath', '/dev/null')
    env = os.environ.copy()
    if not real:
        for name in ARTIFACTS[2:]:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text('{"fixture_sources": []}\n')
        bin_dir = root.parent / (root.name + '-bin')
        bin_dir.mkdir()
        cargo = bin_dir / 'cargo'
        cargo.write_text(SNAPSHOT_FIXTURE)
        cargo.chmod(0o755)
        env['PATH'] = str(bin_dir) + os.pathsep + env['PATH']
    regenerate(root, env)
    git(root, 'add', '.')
    git(root, 'commit', '-qm', 'Fixture base')
    return env


def regenerate(root, env):
    return run(root, sys.executable, 'scripts/derived', 'regenerate', env=env)


class DerivedTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='derived-test-', dir=os.environ.get('TMPDIR'))
        self.root = Path(self.tmp.name) / 'repo'
        self.env = fixture(self.root)

    def tearDown(self):
        self.tmp.cleanup()

    def test_corruption_rejected_before_healing_every_artifact(self):
        for artifact in ARTIFACTS:
            for corrupt in (b'{bad json', b'<<<<<<< ours\n{}\n=======\n{}\n>>>>>>> theirs\n'):
                with self.subTest(artifact=artifact, corrupt=corrupt):
                    path = self.root / artifact
                    good = path.read_bytes()
                    path.write_bytes(corrupt)
                    result = run(self.root, sys.executable, 'scripts/check-derived', env=self.env, check=False)
                    self.assertNotEqual(result.returncode, 0, 'frontdoor must reject corrupt ' + artifact)
                    self.assertEqual(path.read_bytes(), corrupt, 'validation must not heal corruption')
                    path.write_bytes(good)

    def test_explicit_regeneration_recovers_corruption_every_artifact(self):
        for name in ARTIFACTS:
            (self.root / name).write_text('<<<<<<< corrupt\n')
        regenerate(self.root, self.env)
        for name in ARTIFACTS:
            self.assertIsInstance(json.loads((self.root / name).read_bytes()), dict)
        self.assertEqual((self.root / ARTIFACTS[2]).read_bytes(), (self.root / ARTIFACTS[3]).read_bytes())

    def test_snapshot_bootstrap_restores_exact_head_bytes(self):
        embedded = self.root / ARTIFACTS[3]
        expected = git(self.root, 'show', 'HEAD:' + ARTIFACTS[3]).stdout.encode()
        cargo = self.root.parent / (self.root.name + '-bin') / 'cargo'
        cargo.write_text(SNAPSHOT_FIXTURE.replace(
            'values = sorted',
            f'assert (root / {ARTIFACTS[3]!r}).read_bytes() == {expected!r}, "bootstrap must equal HEAD bytes"\nvalues = sorted'))
        for missing in (False, True):
            with self.subTest(missing=missing):
                if missing:
                    embedded.unlink()
                else:
                    embedded.write_text('{broken')
                regenerate(self.root, self.env)
                self.assertEqual(embedded.read_bytes(), expected)

    def test_dirty_stale_artifact_not_exempt(self):
        path = self.root / 'DIVERGENCES.json'
        value = json.loads(path.read_text())
        value['counts']['rows'] = -100
        path.write_text(json.dumps(value))
        result = run(self.root, sys.executable, 'scripts/check-derived', env=self.env, check=False)
        self.assertNotEqual(result.returncode, 0, 'dirty stale derived artifact must not be exempt')
        self.assertNotEqual(json.loads(path.read_text())['counts']['rows'], -100)

    def test_failed_generator_rejected(self):
        (self.root / 'tools/divergence_index.py').write_text('raise SystemExit(37)\n')
        result = run(self.root, sys.executable, 'scripts/check-derived', env=self.env, check=False)
        self.assertNotEqual(result.returncode, 0, 'generator exit status must be checked')
        self.assertIn('37', result.stdout)

    def test_generator_json_and_marker_output_rejected(self):
        for payload in (b'{bad', b'<<<<<<< generator\n'):
            (self.root / 'tools/divergence_index.py').write_text(f'from pathlib import Path\nPath("DIVERGENCES.json").write_bytes({payload!r})\n')
            result = run(self.root, sys.executable, 'scripts/check-derived', env=self.env, check=False)
            self.assertNotEqual(result.returncode, 0, 'successful generator must not bypass JSON/marker validation')
            copy(self.root, 'tools/divergence_index.py')
            regenerate(self.root, self.env)

    def test_primitive_identity_reaches_commit_fixed_point(self):
        source = self.root / 'crates/romacs-vm/src/prim.rs'
        source.write_bytes(source.read_bytes() + b'\n')
        regenerate(self.root, self.env)
        before = (self.root / 'PRIMITIVES.json').read_bytes()
        git(self.root, 'add', '.')
        git(self.root, 'commit', '-qm', 'Commit input and generated output')
        regenerate(self.root, self.env)
        self.assertEqual(before, (self.root / 'PRIMITIVES.json').read_bytes(), 'primitive index must reach commit fixed point')

    def test_sources_first_merge(self):
        exercise_merge(self.root, self.env)

    def test_native_merge_fails_closed_then_finish(self):
        prepare_branches(self.root, self.env)
        result = git(self.root, 'merge', '--no-ff', 'derived-right', check=False, env=self.env)
        self.assertNotEqual(result.returncode, 0, 'native merge must defer, not accept arbitrary artifact bytes')
        self.assertIn('PRIMITIVES.json', git(self.root, 'diff', '--name-only', '--diff-filter=U').stdout)
        run(self.root, sys.executable, 'scripts/derived', 'finish', env=self.env)
        assert_merged(self.root)

    def test_finish_rejects_unstaged_or_untracked_sources(self):
        prepare_branches(self.root, self.env)
        git(self.root, 'merge', '--no-ff', 'derived-right', check=False, env=self.env)
        source = self.root / 'crates/romacs-vm/src/prim.rs'
        before = source.read_bytes()
        for untracked in (False, True):
            with self.subTest(untracked=untracked):
                if untracked:
                    added = self.root / 'crates/romacs-vm/src/prims/extra.rs'
                    added.write_text('defsubr! { name = "never-committed" }\n')
                else:
                    source.write_bytes(before + b'\n')
                result = run(self.root, sys.executable, 'scripts/derived', 'finish', env=self.env, check=False)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('stage all source resolutions', result.stdout)
                if untracked:
                    added.unlink()
                else:
                    source.write_bytes(before)
        run(self.root, sys.executable, 'scripts/derived', 'finish', env=self.env)
        assert_merged(self.root)

    def test_source_conflict_is_not_resolved_or_committed(self):
        prepare_branches(self.root, self.env, source_conflict=True)
        before = git(self.root, 'rev-parse', 'HEAD').stdout
        result = run(self.root, sys.executable, 'scripts/derived', 'merge', 'derived-right', env=self.env, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('resolve and stage source conflicts first', result.stdout)
        self.assertEqual(before, git(self.root, 'rev-parse', 'HEAD').stdout)
        self.assertIn('shared-source.txt', git(self.root, 'diff', '--name-only', '--diff-filter=U').stdout)


def prepare_branches(root, env, source_conflict=False):
    run(root, sys.executable, 'scripts/derived', 'setup', env=env)
    if source_conflict:
        (root / 'shared-source.txt').write_text('base\n')
        git(root, 'add', '.')
        git(root, 'commit', '-qm', 'Conflict base')
    base = git(root, 'rev-parse', 'HEAD').stdout.strip()
    for branch in ('left', 'right'):
        git(root, 'checkout', '-qb', 'derived-' + branch, base)
        source = root / 'crates/romacs-vm/src/lib.rs'
        text = source.read_text()
        table_start = text.index('const PRIMITIVES:')
        table_end = text.index('\n];', table_start)
        position = text.index('\n', table_start) + 1 if branch == 'left' else table_end + 1
        source.write_text(text[:position] + f'    ("derived-{branch}", Primitive::Apply),\n' + text[position:])
        ledger = root / 'crates' / ('derived-' + branch) / 'GNU-DIVERGENCES.md'
        ledger.parent.mkdir(parents=True)
        ledger.write_text(f'| derived-{branch} | `derived-{branch}` | gnu | romacs | test |\n')
        (root / f'snapshot-input-{branch}.txt').write_text(branch)
        if source_conflict:
            (root / 'shared-source.txt').write_text(branch + '\n')
        regenerate(root, env)
        git(root, 'add', '.')
        git(root, 'commit', '-qm', 'Source changes on ' + branch)
    git(root, 'checkout', '-q', 'derived-left')


def assert_merged(root):
    primitives = json.loads((root / 'PRIMITIVES.json').read_text())['primitives']
    divergences = json.loads((root / 'DIVERGENCES.json').read_text())['by_symbol']
    assert {'derived-left', 'derived-right'} <= set(primitives), MERGED
    assert {'derived-left', 'derived-right'} <= set(divergences), MERGED
    assert not git(root, 'status', '--porcelain').stdout, 'merge result must be committed and clean'
    assert len(git(root, 'show', '-s', '--format=%P').stdout.split()) == 2, 'must create a genuine merge commit'
    snapshots = [json.loads((root / name).read_bytes()) for name in ARTIFACTS[2:]]
    assert snapshots[0] == snapshots[1], 'both snapshots must be regenerated identically'
    for snapshot in snapshots:
        if 'vm2' in snapshot:
            assert {'derived-left', 'derived-right'} <= set(snapshot['vm2']['entries']), MERGED
    before = {name: (root / name).read_bytes() for name in ARTIFACTS}
    return before


def exercise_merge(root, env, mutant=False):
    prepare_branches(root, env)
    if mutant:
        path = root / 'scripts/derived'
        text = path.read_text()
        anchor = '    regenerate(root)\n    validate(root)\n'
        assert text.count(anchor) == 1, 'mutant must apply exactly once'
        path.write_text(text.replace(anchor, '    validate(root)\n'))
        git(root, 'config', '--local', 'merge.romacs-derived.driver', 'true')
        git(root, 'add', 'scripts/derived')
        git(root, 'commit', '-qm', 'Fixture mutant skips regeneration')
    result = git(root, 'merge-derived', 'derived-right', env=env)
    print(result.stdout, end='')
    before = assert_merged(root)
    regenerate(root, env)
    assert before == {name: (root / name).read_bytes() for name in ARTIFACTS}, 'merge result must equal canonical regeneration'
    if not mutant and 'fixture_sources' in json.loads(before[ARTIFACTS[2]]):
        assert json.loads(before[ARTIFACTS[2]])['fixture_sources'] == ['snapshot-input-left', 'snapshot-input-right'], MERGED


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--real', action='store_true')
    parser.add_argument('--mutant', action='store_true')
    parser.add_argument('--baseline', action='store_true')
    parser.add_argument('--baseline-commit', default='8dabc1a174d6cc1d4ffbda6966c4d56b18432ff9')
    args, extra = parser.parse_known_args()
    if args.baseline:
        with tempfile.TemporaryDirectory(prefix='derived-baseline-', dir=os.environ.get('TMPDIR')) as tmp:
            root = Path(tmp) / 'repo'
            env = fixture(root)
            baseline = git(ROOT, 'show', args.baseline_commit + ':scripts/check-derived').stdout
            assert 'preexisting' in baseline, 'baseline no longer contains dirty-artifact exemption'
            (root / 'scripts/check-derived').write_text(baseline)
            (root / 'tools/primitive_index.py').write_text(git(
                ROOT, 'show', args.baseline_commit + ':tools/primitive_index.py').stdout)
            observed = []
            for name in ARTIFACTS:
                path = root / name
                good = path.read_bytes()
                path.write_text('<<<<<<< corrupt\n')
                result = run(root, sys.executable, 'scripts/check-derived', env=env, check=False)
                if result.returncode == 0:
                    observed.append(name)
                path.write_bytes(good)
            assert observed == [ARTIFACTS[0], *ARTIFACTS[2:]], 'baseline corruption reproduction must hit the known acceptance hole'
            print('BASELINE RED: frontdoor accepted corrupt artifacts: ' + ', '.join(observed))
            primitive = root / 'PRIMITIVES.json'
            primitive.write_text('<<<<<<< corrupt\n')
            failed = run(root, sys.executable, 'tools/primitive_index.py', env=env, check=False)
            assert failed.returncode and 'JSONDecodeError' in failed.stdout, 'primitive generator must reproduce old-output parse failure'
            print('BASELINE RED: primitive generator parses corrupt old output and raises JSONDecodeError')
            copy(root, 'tools/primitive_index.py')
            regenerate(root, env)
            copy(root, 'scripts/check-derived')
            for name in ARTIFACTS:
                path = root / name
                good = path.read_bytes()
                path.write_text('<<<<<<< corrupt\n')
                result = run(root, sys.executable, 'scripts/check-derived', env=env, check=False)
                assert result.returncode and path.read_text() == '<<<<<<< corrupt\n', 'candidate must reject corruption without healing'
                path.write_bytes(good)
            print('CANDIDATE GREEN: same corruption refused before healing for all four artifacts')
            return 0
    if args.real or args.mutant:
        with tempfile.TemporaryDirectory(prefix='derived-merge-', dir=os.environ.get('TMPDIR')) as tmp:
            if args.mutant:
                baseline_root = Path(tmp) / 'baseline'
                baseline_env = fixture(baseline_root, real=args.real)
                exercise_merge(baseline_root, baseline_env)
                print('BASELINE GREEN: source-first merge before applying mutant')
            root = Path(tmp) / 'repo'
            env = fixture(root, real=args.real)
            try:
                exercise_merge(root, env, mutant=args.mutant)
            except AssertionError as exc:
                if args.mutant and str(exc) == MERGED:
                    print('MUTANT KILLED at target assertion: ' + str(exc))
                    return 0
                raise
            if args.mutant:
                raise AssertionError('mutant survived')
            print('PASS: real-generator source-first merge and canonical byte fixed point')
            return 0
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(DerivedTests)
    return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1


if __name__ == '__main__':
    raise SystemExit(main())
