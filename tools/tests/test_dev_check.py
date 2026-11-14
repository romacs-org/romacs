"""tools/dev-check: how a conflict is classed, what the structural run is, and the report."""
import importlib.machinery
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
loader = importlib.machinery.SourceFileLoader('dev_check', str(ROOT / 'tools/dev-check'))
check = importlib.util.module_from_spec(importlib.util.spec_from_loader(loader.name, loader))
loader.exec_module(check)


def marked(ours, base, theirs):
    return '\n'.join(['before', '<<<<<<< beta', *ours, '||||||| 0123abc', *base, '=======', *theirs, '>>>>>>> HEAD', 'after'])


class Kind(unittest.TestCase):
    def test_both_sides_adding_at_one_place_is_additive(self):
        self.assertEqual(check.kind(marked(['a', 'ours'], ['a'], ['a', 'theirs'])), 'additive')
        self.assertEqual(check.kind(marked(['ours'], [], ['theirs'])), 'additive')

    def test_a_side_rewriting_a_base_line_is_an_overlap(self):
        self.assertEqual(check.kind(marked(['a2', 'ours'], ['a'], ['a', 'theirs'])), 'overlap')

    def test_one_overlapping_hunk_makes_the_file_an_overlap(self):
        text = marked(['ours'], [], ['theirs']) + '\n' + marked(['x'], ['a'], ['y'])
        self.assertEqual(check.kind(text), 'overlap')

    def test_a_file_with_no_hunk_conflicts_whole(self):
        self.assertEqual(check.kind('a\nb\n'), 'whole')

    def test_a_file_one_landing_in_twenty_touches_is_hot(self):
        self.assertEqual(check.conflict('a.rs', 'overlap', 7),
                         'conflict a.rs: overlap (7 of the last 150 landings touched it), '
                         'both sides rewrote the same lines; fix: merge the trunk in and resolve it')
        self.assertEqual(check.conflict('a.rs', 'additive', 8),
                         'conflict a.rs: additive in a hot file (8 of the last 150 landings touched it), both sides add at the '
                         "same place; fix: merge the trunk in and keep both sides' lines, then submit at once: it moves often")

    def test_landings_count_each_file_once_a_landing(self):
        touched = check.landings('\0\na.rs\0b.rs\0\0\na.rs\0')
        self.assertEqual((touched['a.rs'], touched['b.rs'], touched['c.rs']), (2, 1, 0))


class Forecast(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.repo = Path(temp.name)
        self.git('init', '-q', '-b', 'beta')
        self.commit('base', {'list.txt': 'a\nz\n', 'code.rs': 'fn f() { 1 }\n', 'gone.rs': 'x\n'})
        self.git('checkout', '-q', '-b', 'lane')

    def git(self, *args):
        # A throwaway fixture: no identity or signing key of the caller's.
        subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', *args],
                       cwd=self.repo, check=True)

    def commit(self, message, files, removed=()):
        for name, text in files.items():
            (self.repo / name).write_text(text)
        for name in removed:
            self.git('rm', '-q', name)
        self.git('add', '.')
        self.git('commit', '-q', '-m', message)

    def test_a_branch_the_trunk_has_not_passed_merges_cleanly(self):
        self.commit('lane', {'new.rs': 'y\n'})
        status, summary, findings = check.forecast(self.repo, 'beta')
        self.assertEqual((status, findings), ('ok', []))
        self.assertIn('0 landing(s) behind', summary)

    def test_each_conflicting_file_is_named_with_its_kind(self):
        self.commit('lane', {'list.txt': 'a\nlane\nz\n', 'code.rs': 'fn f() { 2 }\n', 'gone.rs': 'lane\n'})
        self.git('checkout', '-q', 'beta')
        self.commit('trunk', {'list.txt': 'a\ntrunk\nz\n', 'code.rs': 'fn f() { 3 }\n'}, removed=['gone.rs'])
        self.git('checkout', '-q', 'lane')
        status, summary, findings = check.forecast(self.repo, 'beta')
        self.assertEqual(status, 'FAIL')
        self.assertIn('3 conflict(s) with beta', summary)
        self.assertIn('(1 landing(s) behind)', summary)
        kinds = {finding.split(':')[0]: finding.split(': ')[1].split(' ')[0] for finding in findings}
        self.assertEqual(kinds, {'conflict list.txt': 'additive', 'conflict code.rs': 'overlap', 'conflict gone.rs': 'whole'})
        # The base commit and the trunk's own: two of the trunk's commits touched each file.
        self.assertTrue(all('(2 of the last 150 landings touched it)' in finding for finding in findings))

    def test_a_path_git_would_quote_is_read_as_it_is(self):
        # An overlap in such a file must not read as a file one side deleted.
        name = 'ελληνικά "x".rs'
        self.git('checkout', '-q', 'beta')
        self.commit('base 2', {name: 'fn f() { 1 }\n'})
        self.git('checkout', '-q', 'lane')
        self.git('merge', '-q', 'beta')
        self.commit('lane', {name: 'fn f() { 2 }\n'})
        self.git('checkout', '-q', 'beta')
        self.commit('trunk', {name: 'fn f() { 3 }\n'})
        self.git('checkout', '-q', 'lane')
        self.assertEqual(check.forecast(self.repo, 'beta')[2],
                         [check.conflict(name, 'overlap', 2)])

    def test_uncommitted_work_is_said_to_be_left_out(self):
        (self.repo / 'new.rs').write_text('y\n')
        self.assertIn('1 uncommitted path(s) are not in the forecast', check.forecast(self.repo, 'beta')[2][0])


class Structural(unittest.TestCase):
    def test_each_package_runs_once_with_its_targets_and_rules(self):
        gate = check.load_gate()
        runs = {package: (flags, rule) for package, flags, rule in check.plan(gate, {'crates/romacs-vm/src/gc.rs'}, ROOT)}
        self.assertEqual(set(runs), {binary.split('::')[0] for binary in gate.STRUCTURAL})
        # The rules that run the compiler take minutes: the queue's, not a lane's.
        self.assertTrue(set(gate.SLOW_RULES) <= set(gate.STRUCTURAL))
        self.assertNotIn('compile_boundaries', runs['romacs-vm'][1])
        self.assertNotIn('primitive_gc_boundary', runs['romacs-vm'][1])
        self.assertEqual(runs['romacs-vm'][0][:4], ['--lib', '--bins', '--test', 'all'])
        self.assertIn('binary_id(romacs-vm::all) & test(/^no_remote_references::/)', runs['romacs-vm'][1])
        self.assertEqual(runs['romacs-gui-tests'][0], ['--test', 'harness_contract'])

    def test_a_change_with_no_rust_runs_no_rule(self):
        self.assertEqual(check.plan(check.load_gate(), {'lisp/romacs/romacs-agent.el', 'AGENTS.md'}, ROOT), [])

    def test_a_changed_crate_adds_its_own_rule(self):
        names = {'crates/romacs-display-runtime/src/render_thread/window_events.rs'}
        runs = check.plan(check.load_gate(), names, ROOT)
        self.assertEqual(runs[-1][:2], ('romacs-display-runtime', ['--lib']))
        self.assertIn('time_discipline_test', runs[-1][2])

    def test_a_failing_rule_is_named_with_what_it_printed(self):
        log = '\n'.join([
            '        FAIL [   0.105s] (29/34) romacs-vm::all no_remote_references::cells_only_shrink',
            '  stderr ───',
            "    thread 'no_remote_references::cells_only_shrink' (3029322) panicked at crates/romacs-vm/tests/all/../no_remote_references.rs:857:5:",
            '    Vm.new_field: a cell no census names',
            '    note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace',
            '',
            '        FAIL [   0.105s] (29/34) romacs-vm::all no_remote_references::cells_only_shrink',
        ])
        self.assertEqual(check.failures(log),
                         [('romacs-vm::all no_remote_references::cells_only_shrink', ['Vm.new_field: a cell no census names'])])


class Report(unittest.TestCase):
    def test_failures_come_first_and_decide_the_exit(self):
        lines, status = check.report([('forecast', 'ok', 'clean', []), ('structural', 'FAIL', '1 rule(s) broken', ['a: b'])])
        self.assertEqual(lines, ['structural: FAIL 1 rule(s) broken', '  a: b', 'forecast: ok clean', 'dev-check: FAIL (structural)'])
        self.assertEqual(status, 1)

    def test_a_stage_that_did_not_run_is_not_a_pass(self):
        lines, status = check.report([('forecast', 'ok', 'clean', []), ('structural', 'SKIP', 'cargo is not on PATH', [])])
        self.assertEqual((lines[-1], status), ('dev-check: INCOMPLETE (structural did not run)', 2))

    def test_all_stages_passing_is_ok(self):
        self.assertEqual(check.report([('forecast', 'ok', 'clean', [])]), (['forecast: ok clean', 'dev-check: ok'], 0))


if __name__ == '__main__':
    unittest.main()
