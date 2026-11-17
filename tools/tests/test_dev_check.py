"""tools/dev-check: how a conflict is classed, what the structural run is, and the report."""
import importlib.machinery
import importlib.util
import os
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

    def test_only_exact_renames_count_as_moved(self):
        self.commit('more', {'kept.el': '(a)\n', 'edited.el': '(b)\n(c)\n(d)\n'})
        base = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=self.repo, capture_output=True, text=True).stdout.strip()
        (self.repo / 'lisp').mkdir()
        self.git('mv', 'kept.el', 'lisp/kept.el')
        self.git('mv', 'edited.el', 'lisp/edited.el')
        self.git('commit', '-q', '-m', 'move')
        (self.repo / 'lisp/edited.el').write_text('(b)\n(c)\n(e)\n')
        self.assertEqual(check.moved_unchanged(self.repo, base), {'lisp/kept.el'})

    def test_uncommitted_work_is_said_to_be_left_out(self):
        (self.repo / 'new.rs').write_text('y\n')
        self.assertIn('1 uncommitted path(s) are not in the forecast', check.forecast(self.repo, 'beta')[2][0])


class Lint(unittest.TestCase):
    def test_runtime_lisp_that_is_still_there_is_linted(self):
        names = {'lisp/emacs-lisp/romacs-lint.el', 'lisp/session/romacs-deleted-by-the-change.el', 'lisp/simple.el',
                 'lisp/session/README', 'crates/romacs-vm/src/gc.rs', 'lisp/packages/jabber/jabber.el'}
        self.assertEqual(check.lisp_files(names, ROOT), ['lisp/emacs-lisp/romacs-lint.el', 'lisp/simple.el'])

    def test_a_file_moved_with_its_text_unchanged_is_not_linted(self):
        names = {'lisp/simple.el', 'lisp/emacs-lisp/romacs-lint.el'}
        self.assertEqual(check.lisp_files(names, ROOT, {'lisp/simple.el'}), ['lisp/emacs-lisp/romacs-lint.el'])

    def test_the_loaddefs_stage_runs_only_for_lisp_and_themes(self):
        self.assertEqual(check.loaddefs(ROOT, {'crates/romacs-vm/src/gc.rs', 'lisp/GENERATED'}),
                         ('ok', 'no Lisp or theme file changed', []))

    def test_the_lint_is_given_the_base_the_paths_came_from(self):
        # Left to itself the lint compares with beta, whatever trunk was asked for.
        command = check.lint_command(ROOT, ['lisp/session/a.el'], 'abc123', '/x/romacs-vm-gui')
        self.assertEqual(command[2:], [str(ROOT / 'tools/romacs-lint'), '--base', 'abc123', '--binary', '/x/romacs-vm-gui', 'lisp/session/a.el'])
        self.assertNotIn('--binary', check.lint_command(ROOT, ['lisp/session/a.el'], 'abc123', None))

    def test_the_lints_closing_line_is_the_summary(self):
        output = "lisp/session/a.el:3: error: in f: `g' is not defined; fix: require its library\nromacs-lint: 1 file, 1 error, 0 warnings, 0 hints new since abc\n"
        self.assertEqual(check.lint_stage(1, output, ''),
                         ('FAIL', '1 file, 1 error, 0 warnings, 0 hints new since abc',
                          ["lisp/session/a.el:3: error: in f: `g' is not defined; fix: require its library"]))
        self.assertEqual(check.lint_stage(0, 'romacs-lint: 1 file, 0 errors, 0 warnings, 0 hints\n', ''),
                         ('ok', '1 file, 0 errors, 0 warnings, 0 hints', []))

    def test_a_lint_that_gave_no_result_is_not_a_pass(self):
        status, summary, findings = check.lint_stage(2, '', 'usage: romacs-lint\nromacs-lint: error: no host at /x/romacs-vm-gui\n')
        self.assertEqual((status, summary), ('SKIP', 'tools/romacs-lint gave no result'))
        self.assertIn('no host at /x/romacs-vm-gui; fix: pass --binary HOST', findings[0])


def answer(packages=(), files=(), everything=False, harnesses=()):
    """An answer of impact.select's shape, as the tests stage reads it."""
    return {'everything': everything, 'reasons': ['Cargo.lock'] if everything else [], 'packages': {p: 'changed' for p in packages},
            'files': {f: 'changed' for f in files}, 'harnesses': {h: 'changed' for h in harnesses}}


class Toolchain(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        for name in ('rust/cargo', 'nextest/cargo-nextest', 'both/cargo', 'both/cargo-nextest'):
            path = self.root / name
            path.parent.mkdir(exist_ok=True)
            path.write_text('#!/bin/sh\n')
            path.chmod(0o755)

    def test_a_path_that_has_both_is_kept(self):
        both = str(self.root / 'both')
        self.assertEqual(check.toolchain_path({'PATH': both, 'ROMACS_TOOLCHAIN_BIN': '/x', 'CARGO_HOME': '/h'}),
                         os.pathsep.join([both, '/h/bin']))

    def test_the_pins_complete_a_path_that_lacks_them(self):
        env = {'PATH': '/usr/bin', 'ROMACS_TOOLCHAIN_BIN': str(self.root / 'rust'), 'ROMACS_NEXTEST_BIN': str(self.root / 'nextest'),
               'HOME': '/u'}
        self.assertEqual(check.toolchain_path(env),
                         os.pathsep.join(['/usr/bin', str(self.root / 'rust'), str(self.root / 'nextest'), '/u/.cargo/bin']))

    def test_cargos_home_comes_last_so_it_does_not_win(self):
        # Cargo puts its home's bin before PATH only when PATH lacks it.
        both = str(self.root / 'both')
        self.assertEqual(check.toolchain_path({'PATH': os.pathsep.join(['/h/bin', both]), 'CARGO_HOME': '/h'}),
                         os.pathsep.join(['/h/bin', both]))

    def test_the_stage_runs_on_the_toolchains_path_and_puts_its_callers_back(self):
        seen, keep = [], {name: os.environ.get(name) for name in ('PATH', 'ROMACS_TOOLCHAIN_BIN', 'ROMACS_NEXTEST_BIN', 'CARGO_HOME')}
        real = check.run_tests
        check.run_tests = lambda root, names, threads: seen.append(os.environ['PATH']) or ('ok', 'ran', [])
        try:
            os.environ.update(PATH='/nowhere', ROMACS_TOOLCHAIN_BIN=str(self.root / 'rust'),
                              ROMACS_NEXTEST_BIN=str(self.root / 'nextest'), CARGO_HOME='/h')
            self.assertEqual(check.tests(ROOT, [], 1), ('ok', 'ran', []))
            self.assertEqual(seen, [os.pathsep.join(['/nowhere', str(self.root / 'rust'), str(self.root / 'nextest'), '/h/bin'])])
            self.assertEqual(os.environ['PATH'], '/nowhere')
            del os.environ['ROMACS_TOOLCHAIN_BIN']
            self.assertEqual(check.tests(ROOT, [], 1)[0], 'SKIP')
            self.assertEqual((os.environ['PATH'], len(seen)), ('/nowhere', 1))
        finally:
            check.run_tests = real
            for name, value in keep.items():
                if value is None:
                    os.environ.pop(name, None)
                else:
                    os.environ[name] = value

    def test_the_tests_see_no_running_editors_launch_variables(self):
        # A check started from inside Romacs: the session's source
        # directory steered source-directory in the VM's tests.
        env = check.test_environment('/scratch', {'PATH': '/usr/bin', 'ROMACS_SOURCE_DIRECTORY': '/elsewhere',
                                                  'ROMACS_RUNTIME_ROOT': '/build', 'WAYLAND_DISPLAY': 'wayland-1',
                                                  'ROMACS_TOOLCHAIN_BIN': '/rust'})
        self.assertEqual(env, {'PATH': '/usr/bin', 'ROMACS_TOOLCHAIN_BIN': '/rust', 'TMPDIR': '/scratch'})

    def test_neither_way_is_no_toolchain(self):
        self.assertIsNone(check.toolchain_path({'PATH': '/usr/bin'}))
        # cargo without nextest runs no test.
        self.assertIsNone(check.toolchain_path({'PATH': '/usr/bin', 'ROMACS_TOOLCHAIN_BIN': str(self.root / 'rust')}))


class Tests(unittest.TestCase):
    def test_a_change_that_reaches_no_package_runs_the_quick_rules(self):
        runs = {package: (flags, rule) for package, flags, rule in check.plan(answer(), ROOT)}
        self.assertEqual(set(runs), {binary.split('::')[0] for binary in check.impact.STRUCTURAL})
        # The rules that run the compiler take minutes: the queue's, not a lane's.
        self.assertTrue(set(check.impact.SLOW_RULES) <= set(check.impact.STRUCTURAL))
        self.assertNotIn('compile_boundaries', runs['romacs-vm'][1])
        self.assertNotIn('primitive_gc_boundary', runs['romacs-vm'][1])
        self.assertIn('no_remote_references', runs['romacs-vm'][1])
        self.assertEqual(runs['romacs-vm'][0][:4], ['--lib', '--bins', '--test', 'all'])

    def test_no_run_takes_the_tests_that_need_the_gates_evidence(self):
        for _, _, rule in check.plan(answer(packages=['romacs-vm', 'romacs-gnu-tests']), ROOT):
            self.assertTrue(rule.endswith(f') & not {check.impact.GATE_ONLY}'), rule)
        self.assertIn('parity_ratchet_', check.impact.GATE_ONLY)

    def test_a_reached_package_runs_whole_but_for_the_slow_rules(self):
        runs = {package: rule for package, _, rule in check.plan(answer(packages=['romacs-vm']), ROOT)}
        self.assertTrue(runs['romacs-vm'].startswith('((package(romacs-vm)) & not ('), runs['romacs-vm'])
        self.assertIn('compile_boundaries', runs['romacs-vm'])
        self.assertIn('primitive_gc_boundary', runs['romacs-vm'])

    def test_a_reached_crate_adds_its_own_rule(self):
        runs = {package: rule for package, _, rule in check.plan(answer(packages=['romacs-display-runtime']), ROOT)}
        self.assertIn('time_discipline_test', runs['romacs-display-runtime'])

    def test_a_change_that_reaches_everything_has_no_plan(self):
        self.assertIsNone(check.plan(answer(everything=True), ROOT))

    def test_the_summary_names_what_the_queue_runs(self):
        self.assertEqual(check.queue_runs(answer(harnesses=['popup', 'agent'])),
                         'the queue runs compile_boundaries, primitive_gc_boundary, the parity ratchets and the harnesses agent, popup')
        self.assertEqual(check.queue_runs(answer()), 'the queue runs compile_boundaries, primitive_gc_boundary, the parity ratchets')

    def test_a_build_that_failed_says_its_errors_and_where(self):
        log = '\n'.join([
            'error[E0433]: cannot find type `Path` in this scope',
            '   --> crates/romacs-vm/tests/gnu/../treesit.rs:303:16',
            '    |',
            'error[E0425]: cannot find function `bootstrapped` in this scope',
            '   --> crates/romacs-vm/tests/gnu/../treesit.rs:304:18',
            'error: could not compile `romacs-vm` (test "gnu") due to 2 previous errors',
            'error[E0433]: cannot find type `Path` in this scope',
            '   --> crates/romacs-vm/tests/gnu/../treesit.rs:303:16',
        ])
        self.assertEqual(check.build_errors(log), [
            'crates/romacs-vm/tests/gnu/../treesit.rs:303:16: error[E0433]: cannot find type `Path` in this scope',
            'crates/romacs-vm/tests/gnu/../treesit.rs:304:18: error[E0425]: cannot find function `bootstrapped` in this scope',
        ])

    def test_a_failed_test_and_a_failed_build_are_both_said(self):
        log = '\n'.join([
            '        FAIL [   0.1s] (1/9) romacs-vm::all quit_async::c_g_stops',
            "    thread 'quit_async::c_g_stops' (1) panicked at crates/romacs-vm/tests/all/../quit_async.rs:9:5:",
            '    it did not stop',
            '',
            'error[E0433]: cannot find type `Path` in this scope',
            '   --> crates/romacs-gnu-tests/tests/x.rs:3:16',
            'error: test run failed',
        ])
        self.assertEqual(check.failed_runs(log, '9 s'), ('FAIL', '1 test(s) failed of what the change reaches; and a package did not build (9 s)', [
            'romacs-vm::all quit_async::c_g_stops: it did not stop',
            'crates/romacs-gnu-tests/tests/x.rs:3:16: error[E0433]: cannot find type `Path` in this scope; fix: make it compile']))
        self.assertEqual(check.failed_runs('nothing useful', '9 s'), ('FAIL', 'the tests did not build or run (9 s)', ['fix: read the log']))

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
