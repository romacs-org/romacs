#!/usr/bin/env python3
"""Tier-0 contract tests. --measure DIR runs real scratch-fixture acceptance."""
import importlib.machinery
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time
import unittest
from unittest.mock import patch
import tempfile

HERE = Path(__file__).resolve().parent
T0 = importlib.machinery.SourceFileLoader('tier0', str(HERE / 'tier0')).load_module()


class Contracts(unittest.TestCase):
    def test_graph(self):
        def graph(child):
            return {'packages': [{'id': n, 'name': n, 'targets': []} for n in ['b', child]],
                    'resolve': {'nodes': [{'id': 'b', 'features': [], 'deps':
                                          [{'pkg': child, 'dep_kinds': [{'kind': None}]}]},
                                         {'id': child, 'features': [], 'deps': []}]}}
        self.assertEqual(T0.check_graph(graph('a'), ['b'], {'a': [], 'b': ['a']}), {'b': []})
        with self.assertRaisesRegex(ValueError, 'links-old-core'):
            T0.check_graph(graph('romacs-core'), ['b'], {'b': []})
        with self.assertRaisesRegex(ValueError, 'edge'):
            T0.check_graph(graph('a'), ['b'], {'b': [], 'a': []})
        inherited = graph('xtask')
        inherited['workspace_members'] = ['b', 'xtask']
        with self.assertRaisesRegex(ValueError, 'links-old-core'):
            T0.check_graph(inherited, ['b'], {'b': []})
        macro = graph('a')
        macro['packages'][0]['targets'] = [{'kind': ['proc-macro']}]
        with self.assertRaisesRegex(ValueError, 'edge'):
            T0.check_graph(macro, ['b'], {'b': ['a'], 'a': []})

    def test_empty_and_incomplete(self):
        listing = {'rust-suites': {'b': {'package-name': 'b', 'binary-name': 'b', 'testcases': {'check': {'filter-match': {'status': 'matches'}, 'ignored': False}}}}}
        with self.assertRaisesRegex(ValueError, 'empty-selection'):
            T0.reconcile({'rust-suites': {}}, [])
        with self.assertRaisesRegex(ValueError, 'incomplete-tests'):
            T0.reconcile(listing, [])
        self.assertEqual(T0.reconcile(listing, [{'type': 'test', 'event': 'ok', 'name': 'b::b$check'}])[0], 1)

    def test_metadata_invalidation(self):
        with tempfile.TemporaryDirectory(dir=Path.home() / '.hermes/cache/scratch') as directory:
            root = Path(directory)
            manifest = root / 'Cargo.toml'
            manifest.write_text('[workspace]\nmembers=[]\n')
            metadata = {'packages': [{'manifest_path': str(manifest)}]}
            with patch.object(T0.GATE, 'out', return_value=json.dumps(metadata).encode()) as cargo:
                env = {'CARGO_HOME': str(root)}
                T0.metadata_for(root, root, env)
                (root / 'lib.rs').write_text('pub fn edited() {}')
                T0.metadata_for(root, root, env)
                self.assertEqual(cargo.call_count, 1)
                (root / 'build.rs').write_text('fn main() {}')
                T0.metadata_for(root, root, env)
                self.assertEqual(cargo.call_count, 2)
                manifest.write_text('[workspace]\nmembers=[]\nresolver="2"\n')
                T0.metadata_for(root, root, env)
                self.assertEqual(cargo.call_count, 3)
                child = root / 'member'
                child.mkdir()
                (child / 'Cargo.toml').write_text('[package]\nname="member"\n')
                T0.metadata_for(root, root, env)
                self.assertEqual(cargo.call_count, 4)
                (root / 'Cargo.lock').write_text('version = 4\n')
                T0.metadata_for(root, root, env)
                self.assertEqual(cargo.call_count, 5)

    def test_metadata_race(self):
        # Reviewer schedule: editor saves optional core feature after metadata.
        with tempfile.TemporaryDirectory(dir=Path.home() / '.hermes/cache/scratch') as directory:
            root = Path(directory)
            manifest = root / 'Cargo.toml'
            manifest.write_text('[workspace]\nmembers=[]\n')
            metadata = {'packages': [{'manifest_path': str(manifest)}]}
            def save(*args, **kwargs):
                manifest.write_text('[workspace]\nmembers=[]\nresolver="2"\n')
                return json.dumps(metadata).encode()
            with patch.object(T0.GATE, 'out', side_effect=save):
                with self.assertRaisesRegex(ValueError, 'metadata-input-changed'):
                    T0.metadata_for(root, root, {'CARGO_HOME': str(root)})
            self.assertFalse((root / 'metadata.json').exists())

    def test_real_names(self):
        edges = T0.tomllib.loads(T0.EDGES.read_text())['edges']
        def graph(parent, child, kind=None):
            return {'workspace_members': [parent, child],
                    'packages': [{'id': n, 'name': n, 'targets': []} for n in (parent, child)],
                    'resolve': {'nodes': [{'id': parent, 'features': [], 'deps':
                        [{'pkg': child, 'dep_kinds': [{'kind': kind}]}]},
                        {'id': child, 'features': [], 'deps': []}]}}
        T0.check_graph(graph('romacs-mem', 'romacs-chars'), ['romacs-mem'], edges)
        T0.check_graph(graph('romacs-chars', 'romacs-gnudiff', 'dev'), ['romacs-chars'], edges)
        for parent, child in [('romacs-chars', 'romacs-core'), ('romacs-chars', 'romacs-mem'),
                              ('romacs-chars', 'romacs-gnudiff')]:
            with self.assertRaises(ValueError):
                T0.check_graph(graph(parent, child), [parent], edges)
        with self.assertRaisesRegex(ValueError, 'links-old-core'):
            T0.check_graph(graph('romacs-redisplay', 'romacs-display-protocol'), ['romacs-redisplay'], edges)
        future = dict(edges, **{'romacs-redisplay': edges['romacs-redisplay'] + ['romacs-display-protocol']})
        T0.check_graph(graph('romacs-redisplay', 'romacs-display-protocol'), ['romacs-redisplay'], future)
        transitive = graph('romacs-redisplay', 'romacs-display-protocol')
        transitive['packages'].append({'id': 'romacs-core', 'name': 'romacs-core', 'targets': []})
        transitive['workspace_members'].append('romacs-core')
        transitive['resolve']['nodes'][1]['deps'] = [{'pkg': 'romacs-core', 'dep_kinds': [{'kind': None}]}]
        transitive['resolve']['nodes'].append({'id': 'romacs-core', 'features': [], 'deps': []})
        with self.assertRaisesRegex(ValueError, 'links-old-core'):
            T0.check_graph(transitive, ['romacs-redisplay'], future)
        with self.assertRaisesRegex(ValueError, 'links-old-core'):
            T0.check_graph(graph('romacs-chars', 'romacs-display-protocol'), ['romacs-chars'], edges)

    def test_worktree_targets(self):
        self.assertEqual(T0.target_for(Path('/a')), T0.target_for(Path('/a')))
        self.assertNotEqual(T0.target_for(Path('/a')), T0.target_for(Path('/b')))


def measure(directory):
    directory = Path(directory).resolve()
    directory.mkdir(parents=True, exist_ok=True)
    root = directory / 'fixture'
    if root.exists():
        raise ValueError('use a fresh evidence directory')
    (root / 'scripts').mkdir(parents=True)
    for script in ('tier0', 'gate'):
        shutil.copyfile(HERE / script, root / 'scripts' / script)
    shutil.copyfile(HERE / 'crate-edges.toml', root / 'scripts/crate-edges.toml')
    (root / 'Cargo.toml').write_text('[workspace]\nmembers = ["romacs-chars", "romacs-mem", "romacs-eval", "romacs-gnudiff", "romacs-core"]\nresolver = "2"\n')
    for crate in ('romacs-chars', 'romacs-mem', 'romacs-eval', 'romacs-gnudiff', 'romacs-core'):
        src = root / crate / 'src'
        src.mkdir(parents=True)
        deps = 'romacs-chars = { path = "../romacs-chars" }' if crate == 'romacs-mem' else 'romacs-core = { path = "../romacs-core" }' if crate == 'romacs-eval' else ''
        (src.parent / 'Cargo.toml').write_text(f'[package]\nname="{crate}"\nversion="0.1.0"\nedition="2024"\n[lib]\ndoctest=false\n[dependencies]\n{deps}\n')
        (src / 'lib.rs').write_text('pub fn value() -> i32 { 0 }\n')
    source = root / 'romacs-mem/src/lib.rs'
    def edit(n):
        # Real production-body edit, not a comment or test-only change.
        source.write_text(f'pub fn value() -> i32 {{ {n} - {n} }}\n#[cfg(test)] mod tests {{ #[test] fn check() {{ assert_eq!(super::value(), romacs_chars::value()); }} }}\n')
    chars_manifest = root / 'romacs-chars/Cargo.toml'
    chars_manifest.write_text(chars_manifest.read_text() + '[dev-dependencies]\nromacs-gnudiff={path="../romacs-gnudiff"}\n')
    edit(0)
    def git(*args):
        subprocess.run(['git', *args], cwd=root, check=True, stdout=subprocess.PIPE)
    git('init', '-b', 'master')
    subprocess.run([T0.GATE.SLOT, 'r4-tier0', '--light', '--', 'cargo', 'generate-lockfile', '--offline'], cwd=root, env=T0.GATE.env(), check=True)
    git('add', '.')
    git('-c', 'commit.gpgsign=false', 'commit', '-m', 'fixture')
    records = []
    def run(label, *args, expected='GREEN'):
        start = time.monotonic()
        p = subprocess.run([sys.executable, str(root / 'scripts/tier0'), *args], cwd=root,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (directory / (label + '.log')).write_text(p.stdout)
        line = next(l for l in p.stdout.splitlines() if l.startswith('tier0: receipt '))
        receipt = Path(line.removeprefix('tier0: receipt '))
        result = json.loads(receipt.read_text())
        result.update(label=label, receipt=str(receipt), measured_s=time.monotonic()-start)
        records.append(result)
        (directory / 'results.json').write_text(json.dumps(records, indent=2)+'\n')
        assert result['verdict'] == expected and (p.returncode == 0) == (expected == 'GREEN'), result
        return result
    run('cold', 'romacs-mem')
    for n in range(1, 11):
        edit(n)
        run('edit-' + str(n), 'romacs-mem')
    values = sorted(r['measured_s'] for r in records if r['label'].startswith('edit-'))
    assert values[9] < 10, values  # nearest-rank p95 for N=10
    old = run('old-core', 'romacs-eval', expected='RED')
    assert old['reason'].startswith('links-old-core') and old['build_wall_s'] == 0
    run('empty', 'romacs-mem', '-E', 'test(nonexistent)', expected='RED')
    run('zero-crate', 'romacs-chars', expected='RED')
    chars_source = root / 'romacs-chars/src/lib.rs'
    chars_source.write_text('pub fn value() -> i32 { 0 }\n#[cfg(test)] mod tests { #[test] fn harness() { assert_eq!(romacs_gnudiff::value(), 0); } }\n')
    run('dev-harness', 'romacs-chars')
    chars_source.write_text('pub fn value() -> i32 { 0 }\n')
    before = chars_manifest.read_text()
    chars_manifest.write_text(before.replace('[dependencies]', '[dependencies]\nromacs-eval={path="../romacs-eval"}'))
    subprocess.run([T0.GATE.SLOT, 'r4-tier0', '--light', '--', 'cargo', 'generate-lockfile', '--offline'], cwd=root, env=T0.GATE.env(), check=True)
    edge = run('upward-edge', 'romacs-chars', expected='RED')
    assert edge['reason'].startswith('edge') and edge['build_wall_s'] == 0
    chars_manifest.write_text(before)
    subprocess.run([T0.GATE.SLOT, 'r4-tier0', '--light', '--', 'cargo', 'generate-lockfile', '--offline'], cwd=root, env=T0.GATE.env(), check=True)
    run('changed', '--changed')
    second = directory / 'second'
    git('worktree', 'add', '-b', 'second', str(second))
    # The committed fixture also has the allowed edge table.
    p = subprocess.run([sys.executable, str(second / 'scripts/tier0'), 'romacs-mem'], cwd=second,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    (directory / 'second.log').write_text(p.stdout)
    receipt = Path(next(l for l in p.stdout.splitlines() if l.startswith('tier0: receipt ')).removeprefix('tier0: receipt '))
    other = json.loads(receipt.read_text())
    assert other['verdict'] == 'GREEN' and other['target'] != records[0]['target'], other
    source.write_text('pub fn value() -> i32 { 0 }\n#[cfg(test)] mod tests { #[test] fn alloc() { let mut x = vec![0u8; 4usize << 30]; for b in &mut x { *b = 1; } std::hint::black_box(x); } }\n')
    run('memory-cap', 'romacs-mem', expected='RED')
    summary = {'rounds': 10, 'p95_s': values[9], 'second_receipt': str(receipt), 'target': records[0]['target']}
    (directory / 'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(json.dumps(summary))


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--measure':
        measure(sys.argv[2])
    else:
        unittest.main()
