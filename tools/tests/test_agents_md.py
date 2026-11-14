"""tools/lib/agents_md.py: what an AGENTS.md names must exist, a module's file has its form,
and the files over a path are found."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/lib'))
import agents_md  # noqa: E402


class Named(unittest.TestCase):
    def test_a_span_names_its_paths_files_targets_tests_and_harnesses(self):
        self.assertEqual(agents_md.named('python3 tools/lib/smells.py resolve'), [('path', 'tools/lib/smells.py')])
        self.assertEqual(agents_md.named('Cargo.lock'), [('file', 'Cargo.lock')])
        self.assertEqual(agents_md.named('make gate-small BASE=x'), [('make', 'gate-small')])
        self.assertEqual(agents_md.named('cargo test --profile gate -p romacs-vm --test quit_async'), [('test', 'romacs-vm', 'quit_async')])
        self.assertEqual(agents_md.named('scaling_live'), [('harness', 'scaling_live')])
        self.assertEqual(agents_md.named('lisp/romacs/'), [('path', 'lisp/romacs/')])

    def test_a_make_commands_options_and_variables_are_no_targets(self):
        self.assertEqual(agents_md.named('make -j 2 check'), [('make', 'check')])
        self.assertEqual(agents_md.named('make --jobs 2 --load-average 1.5 gate-small'), [('make', 'gate-small')])
        self.assertEqual(agents_md.named('make -j check'), [('make', 'check')])
        self.assertEqual(agents_md.named('make -k -C crates check SELECTOR="(tag fast)" JOBS=4'), [('make', 'check')])
        self.assertEqual(agents_md.named("make check-gnosis SELECTOR='(not (tag slow))'"), [('make', 'check-gnosis')])

    def test_a_plain_word_names_nothing(self):
        for span in ('ROMACS_BASE', 'beta', 'declare-function', '// SAFETY:', 'allow(dead_code|unused*)', "-E 'test(/name/)'"):
            self.assertEqual(agents_md.named(span), [], span)

    def test_spans_are_read_outside_fenced_blocks(self):
        text = 'a `one` and `two`\n```\n`fenced`\n```\n`three`\n'
        self.assertEqual(agents_md.spans(text), [(1, 'one'), (1, 'two'), (5, 'three')])

    def test_a_pattern_rule_is_a_target(self):
        targets = agents_md.make_targets('.PHONY: a\na:\n\tx\ncheck-%: b\n\ty\n')
        self.assertTrue(any(target.match('check-gnosis') for target in targets))
        self.assertTrue(any(target.match('a') for target in targets))
        self.assertFalse(any(target.match('gone') for target in targets))


class Problems(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        for name in ('crates/a/tests/held.rs', 'crates/a/tests/suite/main.rs', 'crates/a/tests/bin/helper.rs', 'crates/a/src/lib.rs',
                     'crates/romacs-vm-gui/harness/popup_live.py', 'tools/gate', 'Cargo.lock', 'etc/DOC'):
            (self.root / name).parent.mkdir(parents=True, exist_ok=True)
            (self.root / name).write_text('')
        (self.root / 'crates/a/Cargo.toml').write_text('[package]\nname = "a"\n\n[[test]]\nname = "declared"\npath = "elsewhere.rs"\n')
        self.tree = agents_md.Tree(targets=agents_md.make_targets('gate-small:\n'),
                                   names=frozenset({'held.rs', 'lib.rs', 'Cargo.lock', 'Cargo.toml', 'gate', 'DOC'}),
                                   extensions=frozenset({'.rs', '.lock', '.toml', '.py'}),
                                   placeholders=frozenset({'FILE', 'MODULE'}))

    def problems(self, file, text):
        return agents_md.problems(self.root, file, text, self.tree)

    def test_what_exists_is_no_problem(self):
        text = ('`tools/gate`, `crates/a/`, `etc/DOC`, `Cargo.lock`, `lib.rs`, `make gate-small`, `cargo test -p a --test held`, '
                '`cargo test -p a --test suite`, `cargo test -p a --test declared`, `popup_live`, `origin/beta`, '
                '`crates/a/tests/FILE.rs`, `make check-MODULE`, `cargo test -p a --test FILE`, `Vm.field`, `0.0.20`\n')
        self.assertEqual(self.problems('AGENTS.md', text), [])

    def test_each_thing_gone_is_named_with_its_line(self):
        text = ('`tools/gone`\n`gone-tree/guide.md`\n`make gone`\n`cargo test -p a --test gone`\n`cargo test -p a --test bin`\n'
                '`gone_live`\n`src/lib.rs`\n`Cargo.lock.old`\n`etc/GONE`\n')
        self.assertEqual(self.problems('AGENTS.md', text), [
            'AGENTS.md:1: `tools/gone` does not exist; fix: name what replaced it, or drop the line',
            'AGENTS.md:2: `gone-tree/guide.md` does not exist; fix: name what replaced it, or drop the line',
            'AGENTS.md:3: `make gone` is no target of the Makefile (gone); fix: name the target that replaced it',
            'AGENTS.md:4: `cargo test -p a --test gone` names no test target of a (gone); fix: name the test file that holds it',
            'AGENTS.md:5: `cargo test -p a --test bin` names no test target of a (bin); fix: name the test file that holds it',
            'AGENTS.md:6: `gone_live` is no harness under crates/romacs-vm-gui/harness; fix: name the harness that replaced it',
            'AGENTS.md:7: `src/lib.rs` starts at a crate; fix: write it from the repository root',
            'AGENTS.md:8: `Cargo.lock.old` is the name of no file here; fix: name what replaced it, or drop the line',
            'AGENTS.md:9: `etc/GONE` does not exist; fix: name what replaced it, or drop the line',
        ])

    def test_a_modules_file_opens_with_its_directory_and_is_short(self):
        self.assertEqual(self.problems('crates/a/AGENTS.md', '# crates/a\nWhat it is.\n'), [])
        self.assertEqual(self.problems('crates/a/AGENTS.md', '# The a crate\n'),
                         ["crates/a/AGENTS.md:1: a module's file opens with `# crates/a`; fix: make that its first line"])
        long = '# crates/a\n' + 'line\n' * agents_md.LIMIT
        self.assertEqual(self.problems('crates/a/AGENTS.md', long),
                         [f'crates/a/AGENTS.md:{agents_md.LIMIT + 1}: {agents_md.LIMIT + 1} lines, over the {agents_md.LIMIT} a module file '
                          'may have; fix: keep what only this module needs'])
        # The root's file has neither rule.
        self.assertEqual(self.problems('AGENTS.md', '# Romacs\n' + 'line\n' * 200), [])


class Over(unittest.TestCase):
    MODULES = {'AGENTS.md', 'crates/a/AGENTS.md', 'crates/a/src/gc/AGENTS.md', 'tools/AGENTS.md'}

    def test_the_files_over_a_path_come_deepest_first_and_once(self):
        self.assertEqual(agents_md.over(['crates/a/src/gc/mark.rs', 'crates/a/src/lib.rs', 'crates/a/Cargo.toml'], self.MODULES),
                         ['crates/a/src/gc/AGENTS.md', 'crates/a/AGENTS.md'])

    def test_the_roots_file_is_not_a_modules(self):
        self.assertEqual(agents_md.over(['README.md', 'lisp/romacs/x.el'], self.MODULES), [])

    def test_a_path_is_taken_from_the_root_whatever_way_it_was_given(self):
        root = Path('/somewhere/tree')
        self.assertEqual(agents_md.relative(root, 'tools/../lisp/romacs/x.el'), 'lisp/romacs/x.el')
        self.assertEqual(agents_md.relative(root, '/somewhere/tree/tools/gate'), 'tools/gate')
        self.assertIsNone(agents_md.relative(root, '/elsewhere/tools/gate'))
        self.assertIsNone(agents_md.relative(root, '../tree2/x'))

    def test_notes_are_the_files_text_under_their_names(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        subprocess.run(['git', 'init', '-q'], cwd=root, check=True)
        (root / 'tools').mkdir()
        (root / 'AGENTS.md').write_text('# Root\n')
        (root / 'tools/AGENTS.md').write_text('# tools\nThe checks.\n')
        self.assertEqual(agents_md.files(root), ['AGENTS.md', 'tools/AGENTS.md'])
        self.assertEqual(agents_md.notes(root, ['tools/gate']), '== tools/AGENTS.md\n# tools\nThe checks.')
        self.assertEqual(agents_md.notes(root, ['README.md']), 'no module AGENTS.md stands over these paths; the root AGENTS.md holds')
        self.assertEqual(agents_md.notes(root, ['tools/../README.md', str(root / 'tools/x.py')]), '== tools/AGENTS.md\n# tools\nThe checks.')
        # A tracked file that was deleted, staged or not, is no file of the tree.
        (root / 'tools/gone.py').write_text('')
        subprocess.run(['git', 'add', 'tools/gone.py'], cwd=root, check=True)
        (root / 'tools/gone.py').unlink()
        self.assertNotIn('gone.py', agents_md.tree(root, []).names)
        held = agents_md.tree(root, ['a `FILE` and `tools/gate`'])
        self.assertEqual((held.placeholders, 'AGENTS.md' in held.names, '.md' in held.extensions), (frozenset({'FILE'}), True, True))


class Repository(unittest.TestCase):
    def test_every_agents_file_here_names_only_what_exists(self):
        self.assertEqual(agents_md.check(ROOT), [])


if __name__ == '__main__':
    unittest.main()
