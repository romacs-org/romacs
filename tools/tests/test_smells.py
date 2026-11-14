#!/usr/bin/env python3
"""tools/lib/smells.py: what counts as each smell and what declares one, the
lint census's dedup and attribution, the layout rules, and the ratchet that
holds a tree's totals to its merge-base's."""
import contextlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import smells  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def smell(src, long_file_lines=3):
    return smells.file_smells(src, long_file_lines)


class FileSmellsTest(unittest.TestCase):
    def test_allows_count_each_dead_or_unused_lint_name_unless_expected_with_a_reason(self):
        src = '''
#![allow(dead_code)]
#[allow(unused, unused_imports, clippy::unused_self)]
#[expect(dead_code, reason = "allow(dead_code) in a reason")]
#[expect(unused_variables)]
#[cfg_attr(test, allow(unused_mut))]
#[allow(clippy::too_many_lines)]
'''
        self.assertEqual(smell(src, 100)[0]['allows'], 5)

    def test_comments_and_strings_hold_no_allows_and_code_holds_no_todos(self):
        src = '// #[allow(dead_code)] TODO\nlet s = "#[allow(unused)] FIXME";\nlet TODO = 1;\n'
        found = smell(src, 100)[0]
        self.assertEqual((found['allows'], found['todos']), (0, 1))

    def test_todos_are_whole_words(self):
        self.assertEqual(smell('// TODO, FIXME: XXX HACK; TODOs XXXL hack\n', 100)[0]['todos'], 4)

    def test_a_todo_naming_its_tracker_issue_is_declared(self):
        src = '// TODO(#12): a. FIXME(romacs#3) b. TODO(soon) c. HACK (#4) d.\n'
        self.assertEqual(smell(src, 100)[0]['todos'], 2)

    def test_a_long_file_is_over_the_limit_unless_declared(self):
        self.assertEqual(smell('a\nb\nc\n')[0]['long_files'], 0)
        self.assertEqual(smell('a\nb\nc\nd')[0]['long_files'], 1)
        self.assertEqual(smell('// SMELL(long-file): one table\nb\nc\nd')[0]['long_files'], 0)
        self.assertEqual(smell('// SMELL(long-file):\nb\nc\nd')[0]['long_files'], 1, 'a declaration gives its reason')
        self.assertEqual(smell('let s = "SMELL(long-file): x";\nb\nc\nd')[0]['long_files'], 1, 'in a comment')

    def test_a_primitive_is_declared_in_code_only(self):
        src = 'defsubr! { name = "a", gnu = "data.c:1" }\ndefsubr!{name="b"}\n$crate::prim::defsubr!(@emit $name)'
        self.assertEqual(smell(src)[1], 2)
        self.assertEqual(smell('// defsubr! { name = "a" }\nmacro_rules! defsubr { () => {} }')[1], 0)

    def test_a_primitive_outside_prims_is_declared_directly_above(self):
        declared = '// SMELL(primitive): the probe needs its own\n#[cfg(test)]\ndefsubr! { name = "a" }\n'
        self.assertEqual(smell(declared)[1], 0)
        self.assertEqual(smell('// SMELL(primitive): far\n\ndefsubr! { name = "a" }\n')[1], 1)


def message(manifest, code, at=None, level='warning', text='m'):
    return json.dumps({'reason': 'compiler-message', 'manifest_path': manifest,
                       'message': {'level': level, 'code': code and {'code': code}, 'message': text,
                                   'spans': [at or span()]}})


def span(file='src/lib.rs', line=1, called_from=None):
    return {'is_primary': True, 'file_name': file, 'line_start': line, 'column_start': 1,
            'expansion': called_from and {'span': called_from}}


FINISHED = json.dumps({'reason': 'build-finished', 'success': True})


class LintCensusTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root)
        for crate in ('a', 'b'):
            (Path(self.root) / crate).mkdir()
            (Path(self.root) / crate / 'Cargo.toml').write_text(f'[package]\nname = "pkg-{crate}"\n')
        subprocess.run(['git', 'init', '-q', self.root], check=True)
        self.a, self.b = str(Path(self.root) / 'a/Cargo.toml'), str(Path(self.root) / 'b/Cargo.toml')

    def census(self, *lines):
        return smells.lint_census(self.root, [*lines, FINISHED])

    def test_one_count_per_diagnostic_credited_to_its_package(self):
        self.assertEqual(self.census(message(self.a, 'clippy::doc_markdown'),
                                     message(self.a, 'clippy::doc_markdown'),     # a second target
                                     message(self.a, 'clippy::doc_markdown', span(line=2)),
                                     message(self.b, 'dead_code'),
                                     message(self.b, None),                       # "N warnings emitted"
                                     message(self.b, 'dead_code', level='note'),
                                     'Compiling pkg-b'),
                         {'pkg-a': Counter({'clippy::doc_markdown': 2}), 'pkg-b': Counter({'dead_code': 1})})

    def test_one_file_under_several_names_is_one_place(self):
        absolute = str(Path(self.root) / 'a/src/lib.rs')
        self.assertEqual(self.census(message(self.a, 'dead_code', span('a/src/lib.rs')),
                                     message(self.a, 'dead_code', span('a/tests/../src/lib.rs')),
                                     message(self.a, 'dead_code', span(absolute))),
                         {'pkg-a': Counter({'dead_code': 1})})

    def test_a_warning_inside_a_shared_macro_counts_per_call(self):
        core = '/toolchain/library/core/src/macros/mod.rs'
        calls = [message(self.a, 'clippy::float_cmp', span(core, 40, span('a/src/lib.rs', line)))
                 for line in (1, 2, 2)]
        self.assertEqual(self.census(*calls), {'pkg-a': Counter({'clippy::float_cmp': 2})})

    def test_a_failed_or_cut_off_build_counts_nothing(self):
        warning = message(self.a, 'dead_code')
        failed = json.dumps({'reason': 'build-finished', 'success': False})
        for lines in ([warning], [warning, failed], [FINISHED, warning]):
            with self.assertRaises(ValueError):
                smells.lint_census(self.root, lines)


class GeneratedTest(unittest.TestCase):
    def test_a_header_says_generated_unless_declared_or_a_lockfile(self):
        heads = {'a.rs': '// @generated\n', 'b.el': ';; Do not edit.\n',
                 'c.py': '# Generators of things.\n', 'lisp/gnu/d.el': ';; Generated by GNU\n',
                 'e.txt': '# Generated by tools/record\n# SMELL(generated): recorded from pinned GNU\n',
                 'loom/Cargo.lock': '# This file is automatically @generated by Cargo.\n'}
        with tempfile.TemporaryDirectory() as root:
            for name, head in heads.items():
                (Path(root) / name).parent.mkdir(parents=True, exist_ok=True)
                (Path(root) / name).write_text(head)
            self.assertEqual([n for n in heads if smells.says_generated(root, n)], ['a.rs', 'b.el'])


def now(allows=2, todos=1, long_files=1, lint=3, root=('crates', 'tools'), outside=(), legacy=5,
        strays=2, generated=('fixture.txt',), sites=4, undocumented=1, declared=0, crate='a'):
    """A measure of a tree, its smells in CRATE."""
    return {'crates': {crate: Counter(allows=allows, todos=todos, long_files=long_files)},
            'unsafe': {crate: Counter(sites=sites, undocumented=undocumented, declared=declared)},
            'lints': Counter({'clippy::doc_markdown': lint}) if lint else Counter(),
            'layout': {'root': list(root), 'members_outside_crates': list(outside), 'legacy_primitives': legacy,
                       'primitives_outside_prims': strays, 'generated': list(generated)},
            'strays': {}, 'long': [], 'lints_by_crate': {}}


def rises(base, tree):
    return smells.rises(smells.summary(base), smells.summary(tree))


class RatchetTest(unittest.TestCase):
    """rises(the merge-base's summary, the tree's)."""

    def test_the_summary_is_the_workspaces_totals(self):
        tree = now(declared=1)
        tree['crates']['b'] = Counter(allows=1)
        tree['unsafe']['b'] = Counter(sites=2, undocumented=0, declared=1)
        self.assertEqual(smells.summary(tree), {
            'totals': {'allows': 3, 'todos': 1, 'long_files': 1, 'unsafe_sites': 4, 'unsafe_undocumented': 1,
                       'legacy_primitives': 5, 'primitives_outside_prims': 2, 'lint clippy::doc_markdown': 3},
            'generated': ['fixture.txt']})

    def test_at_the_merge_base(self):
        self.assertEqual(rises(now(), now()), [])

    def test_a_drop_needs_no_record(self):
        self.assertEqual(rises(now(), now(allows=0, lint=0, legacy=1, sites=1, undocumented=0, generated=())), [])

    def test_a_rise_fails_and_says_how_to_declare_it(self):
        self.assertEqual(rises(now(), now(allows=3, lint=5, legacy=6, sites=5, undocumented=2)), [
            'allows: 3, above the 2 of its merge-base; remove 1, or declare it as `#[expect(LINT, reason = "...")]`',
            'legacy_primitives: 6, above the 5 of its merge-base; remove 1, or declare it with `defsubr!` in '
            'crates/romacs-vm/src/prims/',
            'lint clippy::doc_markdown: 5, above the 3 of its merge-base; remove 2, or declare it as '
            '`#[expect(clippy::doc_markdown, reason = "...")]` on the item',
            'unsafe_sites: 5, above the 4 of its merge-base; remove 1, or declare it with `UNSAFE(reason)` beside '
            'its SAFETY: comment',
            'unsafe_undocumented: 2, above the 1 of its merge-base; remove 1, or put a `// SAFETY:` comment on the '
            'line above its `unsafe`'])

    def test_a_declared_unsafe_site_is_not_counted(self):
        self.assertEqual(rises(now(), now(sites=5, declared=1)), [])

    def test_a_new_lint_is_a_rise_from_zero(self):
        tree = now()
        tree['lints']['clippy::new_lint'] = 1
        (problem,) = rises(now(), tree)
        self.assertTrue(problem.startswith('lint clippy::new_lint: 1, above the 0 of its merge-base'))

    def test_moving_a_smell_between_crates_is_no_rise(self):
        self.assertEqual(rises(now(), now(crate='b')), [])

    def test_a_new_generated_file_fails_and_a_gone_one_passes(self):
        self.assertEqual(rises(now(), now(generated=('fixture.txt', 'tools/new_index.json'))),
                         ['generated: tools/new_index.json is new; declare it with `SMELL(generated): reason` in '
                          'its first lines'])
        self.assertEqual(rises(now(), now(generated=())), [])


class RulesTest(unittest.TestCase):
    def rules(self, architecture, **layout):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / smells.ARCHITECTURE).write_text(architecture)
            return smells.rules(root, now(**layout))

    TOP = '# A\n\n| x | `src/` |\n\n## Top level\n\n- `crates/`: Rust; `tools/`.\n\n## Invariants\n\n`logs/`\n'

    def test_every_top_level_entry_is_named_in_its_section(self):
        self.assertEqual(self.rules(self.TOP), [])
        self.assertEqual(self.rules(self.TOP, root=('crates', 'tools', 'logs', 'src')), [
            'top level: logs is not named in the Top level section of ARCHITECTURE.md; name it there',
            'top level: src is not named in the Top level section of ARCHITECTURE.md; name it there'])

    def test_every_member_lives_under_crates(self):
        self.assertEqual(self.rules(self.TOP, outside=('tests/x',)),
                         ['workspace member tests/x is outside crates/; move it there'])

    def test_the_tree_keeps_its_rules(self):
        lay = smells.layout(ROOT, {})
        self.assertEqual(smells.rules(ROOT, {'layout': lay}), [])


class ClippyTreeTest(unittest.TestCase):
    """A checkout's clippy tree names its checkout, and goes when the checkout does."""

    def setUp(self):
        self.base, self.home = Path(tempfile.mkdtemp()), Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.base)
        self.addCleanup(shutil.rmtree, self.home)

    def tree(self, checkout, device=None):
        tree = smells.clippy_tree(self.base, checkout)
        tree.mkdir()
        checkout.parent.mkdir(parents=True, exist_ok=True)
        smells.record_owner(tree, checkout)
        if device is not None:
            (tree / smells.DEVICE).write_text(str(device))
        return tree

    def prune(self):
        with mock.patch.dict(os.environ, {'ROMACS_CLIPPY_TARGET_DIR': str(self.base)}), \
                mock.patch.object(sys, 'argv', ['smells.py', 'prune']), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(smells.main(), 0)
        return sorted(p.name for p in self.base.iterdir() if not p.name.endswith('.lock'))

    def test_only_a_tree_whose_checkout_is_gone_is_pruned(self):
        live, gone = self.home / 'live', self.home / 'gone'
        live.mkdir(), gone.mkdir()
        kept = [self.tree(live), self.base / 'legacy']
        kept[1].mkdir()
        self.tree(gone)
        gone.rmdir()
        self.assertEqual(self.prune(), sorted(p.name for p in kept))

    def test_a_recorded_path_is_read_exactly(self):
        for name in ('live\n', 'cr\r', 'crlf\r\n'):
            checkout = self.home / name
            checkout.mkdir()
            tree = self.tree(checkout)
            self.assertEqual(smells.owner(tree)[0], str(checkout.resolve()))
            self.assertEqual(self.prune(), [tree.name])
            checkout.rmdir()
            self.assertEqual(self.prune(), [])

    def test_a_checkout_whose_disk_is_away_is_kept(self):
        mount = self.home / 'mnt'
        tree = self.tree(mount / 'repo', device=smells.os.stat(self.home).st_dev + 1)
        self.assertEqual(self.prune(), [tree.name], 'the mount point is left, on another device')
        shutil.rmtree(mount)
        self.assertEqual(self.prune(), [tree.name], 'no mount point at all')

    def test_a_directory_not_named_for_its_recorded_checkout_is_kept(self):
        foreign = self.base / 'archive'
        foreign.mkdir()
        (foreign / smells.OWNER).write_text(str(self.home / 'gone'))
        self.assertEqual(self.prune(), ['archive'])

    def test_a_tree_in_use_is_kept(self):
        gone = self.home / 'gone'
        tree = self.tree(gone)
        with smells.tree_lock(self.base, tree.name) as held:
            smells.fcntl.flock(held, smells.fcntl.LOCK_EX)
            self.assertEqual(self.prune(), [tree.name])
        self.assertEqual(self.prune(), [])

    def test_running_clippy_records_the_checkout(self):
        checkout = self.home / 'repo'
        checkout.mkdir()
        done = subprocess.CompletedProcess([], 0, stdout='')
        with mock.patch.dict(os.environ, {'ROMACS_CLIPPY_TARGET_DIR': str(self.base)}), \
                mock.patch.object(smells.subprocess, 'run', return_value=done) as run:
            smells.run_clippy(checkout)
        tree = smells.clippy_tree(self.base, checkout)
        self.assertEqual((tree / smells.OWNER).read_text(), str(checkout.resolve()))
        self.assertEqual(run.call_args.kwargs['env']['CARGO_TARGET_DIR'], str(tree))


class BaseCountsTest(unittest.TestCase):
    """The merge-base is found by git, measured in the kept checkout, and kept.
    A tree's lint count here is the number in its file `n`."""

    def setUp(self):
        self.repo = tempfile.mkdtemp()
        self.cache = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.repo)
        self.addCleanup(shutil.rmtree, self.cache)
        self.measured = []
        for name, value in (('measure', self.measure), ('counting', lambda: self.how),
                            ('cache_dir', lambda: Path(self.cache))):
            patch = mock.patch.object(smells, name, value)
            patch.start()
            self.addCleanup(patch.stop)
        self.how = 'counted one way'
        self.git('init', '-q', '-b', 'trunk')
        (Path(self.repo) / smells.ARCHITECTURE).write_text('## Top level\n\n`crates/`, `tools/`\n')
        self.commit(5)
        self.git('checkout', '-q', '-b', 'branch')

    def git(self, *args):
        return subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@example.org', '-c',
                               'commit.gpgsign=false', *args], cwd=self.repo, check=True,
                              stdout=subprocess.PIPE, text=True).stdout.strip()

    def commit(self, n, name='n'):
        (Path(self.repo) / name).write_text(str(n))
        self.git('add', '-A')
        self.git('commit', '-q', '-m', f'{name} {n}')

    def measure(self, root, clippy_json=None):
        self.measured.append(str(root))
        return now(lint=int((Path(root) / 'n').read_text()))

    def check(self, base='trunk'):
        tree = smells.summary(self.measure(self.repo))
        self.measured.clear()
        return smells.held_to_base(self.repo, base, tree)

    def rise(self, have, had):
        return (f'lint clippy::doc_markdown: {have}, above the {had} of its merge-base; remove {have - had}, '
                'or declare it as `#[expect(clippy::doc_markdown, reason = "...")]` on the item')

    def test_the_branch_is_held_to_its_fork_not_to_the_trunk_since(self):
        self.commit(5, name='other')                      # the branch: unchanged in the lint
        self.git('checkout', '-q', 'trunk')
        self.commit(3)                                    # the trunk drops
        self.git('checkout', '-q', 'branch')
        self.assertEqual(self.check(), [])
        self.assertEqual(self.measured, [str(Path(self.cache) / 'base')])
        self.commit(6)
        self.assertEqual(self.check(), [self.rise(6, 5)])
        self.assertEqual(self.measured, [], 'the merge-base was kept')

    def test_merging_the_trunk_moves_the_merge_base(self):
        self.commit(5, name='other')
        self.git('checkout', '-q', 'trunk')
        self.commit(3)
        self.git('checkout', '-q', 'branch')
        self.git('merge', '-q', '--no-edit', 'trunk')
        self.assertEqual(self.check(), [])
        self.commit(4)
        self.assertEqual(self.check(), [self.rise(4, 3)])

    def test_a_clean_tree_keeps_its_own_counts_for_later_branches(self):
        self.assertEqual(self.check(), [])                # the branch is the trunk's tip
        self.assertEqual(self.measured, [], 'a tree that is its own merge-base is measured once')
        (Path(self.repo) / 'n').write_text('7')           # unstaged: not the tree git has
        self.assertEqual(self.check(), [self.rise(7, 5)])
        self.assertEqual(self.measured, [])

    def test_where_branch_and_trunk_merged_each_other_the_trunks_own_line_is_the_base(self):
        self.commit(5, name='other')                      # the branch's first work, which the trunk will merge
        early = self.git('rev-parse', 'HEAD')
        self.git('checkout', '-q', 'trunk')
        self.commit(3)                                    # the trunk drops to 3
        self.git('checkout', '-q', 'branch')
        self.git('merge', '-q', '--no-edit', 'trunk')     # the branch takes the trunk in: 3
        self.git('checkout', '-q', 'trunk')
        self.git('merge', '-q', '--no-ff', '--no-edit', early)    # and the trunk merges the early work
        self.git('checkout', '-q', 'branch')
        self.assertEqual(len(self.git('merge-base', '--all', 'HEAD', 'trunk').split()), 2)
        self.assertEqual(self.check(), [])                # held to the trunk at 3, not to its own early 5
        self.commit(4)
        self.assertEqual(len(self.check()), 1)

    def test_supplied_diagnostics_are_not_kept_as_a_trees_counts(self):
        smells.held_to_base(self.repo, 'trunk', smells.summary(now()), keep_own=False)
        self.assertEqual(self.measured, [str(Path(self.cache) / 'base')])

    def test_counts_are_kept_per_way_of_counting(self):
        self.commit(5, name='other')
        self.check()
        self.how = 'counted another way'
        self.check()
        self.assertEqual(self.measured, [str(Path(self.cache) / 'base')])

    def test_check_holds_to_the_merge_base(self):
        self.commit(4)                                    # a drop, with no record
        for flags, status in ((['--base', 'trunk'], 0), (['--base', 'no-such-branch'], 1)):
            with mock.patch.object(sys, 'argv', ['smells.py', 'check', '--root', self.repo, *flags]), \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(smells.main(), status, flags)
        self.commit(6)                                    # a rise of one
        with mock.patch.object(sys, 'argv', ['smells.py', 'check', '--root', self.repo, '--base', 'trunk']), \
                contextlib.redirect_stderr(io.StringIO()) as said:
            self.assertEqual(smells.main(), 1)
        self.assertIn(self.rise(6, 5), said.getvalue())

    def test_no_such_trunk_is_an_error(self):
        with self.assertRaises(ValueError):
            self.check(base='no-such-branch')

    def test_a_clone_without_the_local_trunk_uses_origins(self):
        self.commit(6)                                    # a rise of one over the trunk's 5
        self.git('update-ref', 'refs/remotes/origin/trunk', 'trunk')
        self.git('branch', '-q', '-D', 'trunk')
        self.assertEqual(self.check(), [self.rise(6, 5)])


class TwoLanesTest(unittest.TestCase):
    """Two lanes that each add one declared smell merge without a conflict, and
    the merge passes; an undeclared one fails. Measured for real, Clippy aside."""

    LIB = 'pub fn a() {}\n'

    def setUp(self):
        self.repo = Path(tempfile.mkdtemp())
        self.cache = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.repo)
        self.addCleanup(shutil.rmtree, self.cache)
        finished = [json.dumps({'reason': 'build-finished', 'success': True})]
        for patch in (mock.patch.dict(os.environ, {'ROMACS_SMELLS_CACHE': self.cache}),
                      mock.patch.object(smells, 'run_clippy', lambda root: finished),
                      mock.patch.object(smells, 'counting', lambda: 'one way')):
            patch.start()
            self.addCleanup(patch.stop)
        self.write('Cargo.toml', '[workspace]\nmembers = ["crates/romacs-vm"]\n')
        self.write('crates/romacs-vm/Cargo.toml', '[package]\nname = "romacs-vm"\n')
        self.write('crates/romacs-vm/src/lib.rs', 'const PRIMITIVES: &[(&str, Primitive)] = &[\n    ("car", Primitive::Car),\n];\n')
        self.write('crates/romacs-vm/src/prims/data.rs', 'defsubr! { name = "cdr" }\n')
        self.write(smells.ARCHITECTURE, '## Top level\n\n`crates/`, `Cargo.toml`, `ARCHITECTURE.md`\n')
        self.git('init', '-q', '-b', 'trunk')
        self.commit('the trunk')

    def write(self, name, text):
        (self.repo / name).parent.mkdir(parents=True, exist_ok=True)
        (self.repo / name).write_text(text)

    def git(self, *args):
        return subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@example.org', '-c',
                               'commit.gpgsign=false', *args], cwd=self.repo, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True)

    def commit(self, message):
        self.git('add', '-A')
        self.git('commit', '-q', '-m', message)

    def check(self):
        """(check's status, its problems)."""
        with mock.patch.object(sys, 'argv', ['smells.py', 'check', '--root', str(self.repo), '--base', 'trunk']), \
                contextlib.redirect_stderr(io.StringIO()) as said:
            status = smells.main()
        return status, [line for line in said.getvalue().splitlines() if 'measuring the merge-base' not in line]

    def lane(self, name, path, text):
        self.git('checkout', '-q', '-b', name, 'trunk')
        self.write(path, text)
        self.commit(name)
        self.assertEqual(self.check(), (0, []))

    def test_two_declared_smells_merge_without_a_conflict(self):
        self.lane('one', 'crates/romacs-vm/src/one.rs',
                  '// TODO(#7): split the table.\n#[expect(dead_code, reason = "a probe")]\nfn one() {}\n')
        self.lane('two', 'crates/romacs-vm/src/two.rs',
                  '// SAFETY: the probe owns it.\n// UNSAFE(no crate maps it)\nunsafe { two() }\n')
        self.git('checkout', '-q', '-b', 'both', 'trunk')
        for lane in ('one', 'two'):
            done = self.git('merge', '--no-edit', lane)
            self.assertEqual(done.returncode, 0, done.stdout)
        self.assertEqual(self.check(), (0, []))
        self.write('crates/romacs-vm/src/three.rs', '// TODO: undeclared\n')
        self.commit('three')
        self.assertEqual(self.check(), (1, ['smells: todos: 1, above the 0 of its merge-base; remove 1, '
                                            'or name its tracker issue: `TODO(#N)`']))


class CargoSettingsTest(unittest.TestCase):
    def settings(self, home, **env):
        with mock.patch.dict(os.environ, {'CARGO_HOME': home, **env}):
            return smells.cargo_settings()

    def test_what_changes_warnings_changes_the_settings_and_where_a_build_goes_does_not(self):
        home = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, home)
        plain = self.settings(home)
        self.assertEqual(plain, self.settings(home, CARGO_TARGET_DIR='/elsewhere', CARGO_BUILD_JOBS='2'))
        self.assertNotEqual(plain, self.settings(home, CARGO_PROFILE_DEV_DEBUG_ASSERTIONS='false'))
        self.assertNotEqual(plain, self.settings(home, RUSTFLAGS='--cfg x'))
        (Path(home) / 'config.toml').write_text('[profile.dev]\ndebug-assertions = false\n')
        self.assertNotEqual(plain, self.settings(home))


if __name__ == '__main__':
    unittest.main()
