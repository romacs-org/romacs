#!/usr/bin/env python3
"""Tests for tools/gate: VM2 protocol, receipts, environment isolation and run cleanup.

    python3 tools/tests/gate_test.py
"""
import io, contextlib, importlib.machinery, importlib.util, json, multiprocessing, os, pathlib, shutil, subprocess, sys, tempfile, time, unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'lib'))
loader = importlib.machinery.SourceFileLoader('gate', str(pathlib.Path(__file__).resolve().parents[1] / 'gate'))
gate = importlib.util.module_from_spec(importlib.util.spec_from_loader('gate', loader))
loader.exec_module(gate)

FIXTURE_PIN = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'


def pinned_fixture(test, repo):
    root = test.root / 'oracle-root'
    binary = root / FIXTURE_PIN / 'bin/emacs'
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text('fixture ' + FIXTURE_PIN)
    binary.chmod(0o755)
    (repo / 'parity-reference.toml').write_text(f'mirror_commit = "{FIXTURE_PIN}"\n')
    test.enterContext(mock.patch.dict(os.environ, {'GNU_ORACLE_ROOT': str(root)}))
    return binary


FIXTURE_SUITES = {'vm2': ['romacs-vm'], 'display': ['romacs-layout2']}


def metadata_command(*cmd, **kw):
    if cmd[:2] != ('cargo', 'metadata'):
        return
    names = sum(FIXTURE_SUITES.values(), [])
    json.dump({'workspace_members': names, 'packages': [dict(id=n, name=n) for n in names]}, kw['stdout'])


def listed_binaries(cmd, **kw):
    if '--binaries-metadata' in cmd:
        json.dump({'rust-suites': {'pkg': {'package-name': 'pkg', 'binary-name': 'bin',
            'testcases': {'probe': {'ignored': False, 'filter-match': {'status': 'matches'}}}}}}, kw['stdout'])
        return 0
    json.dump({'rust-build-meta': {'fixture': True}, 'rust-binaries': {
        p: {'package-id': p, 'binary-id': p}
        for packages in FIXTURE_SUITES.values() for p in packages
    }}, kw['stdout'])
    return 0


class GraphicalOracleEnvironment(unittest.TestCase):
    def test_full_gate_provisions_pinned_compositor(self):
        environment = gate.test_env(pathlib.Path('/source'), pathlib.Path('/private'), names=set())
        self.assertEqual(environment.get('ROMACS_TEST_LABWC'), gate.LABWC)


class FixtureProvisioning(unittest.TestCase):
    """The gate fills the pinned test-fixture cache before tests, which only read it."""

    def tree_with_fetcher(self, root, status):
        script = root / 'scripts/fetch-test-fixtures'
        script.parent.mkdir(parents=True)
        script.write_text(f'import os, pathlib, sys\n'
                          f'pathlib.Path(sys.argv[0]).with_name("seen").write_text(os.environ["ROMACS_TEST_FONTS_DIR"])\n'
                          f'sys.exit({status})\n')
        return script

    def test_runs_the_tree_fetcher_against_the_shared_cache(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            script = self.tree_with_fetcher(root, 0)
            with mock.patch.dict(os.environ, {'ROMACS_TEST_FONTS_DIR': str(root / 'cache')}):
                gate.provision_fixtures(root)
            self.assertEqual(script.with_name('seen').read_text(), str(root / 'cache'))

    def test_a_failed_fetch_refuses_the_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            self.tree_with_fetcher(root, 1)
            with self.assertRaises(gate.MissingTool):
                gate.provision_fixtures(root)

    def test_a_tree_without_the_fetcher_needs_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            gate.provision_fixtures(pathlib.Path(tmp))


class PortableToolTrust(unittest.TestCase):
    def test_slot_defaults_are_adjacent_even_with_legacy_tools_on_path(self):
        environment = {k: v for k, v in os.environ.items()
                       if k not in ('ROMACS_SLOT', 'ROMACS_SLOT_RUNNER')}
        with mock.patch.dict(os.environ, environment, clear=True), \
             mock.patch.object(shutil, 'which', return_value='/retired/bin/tool'):
            local_loader = importlib.machinery.SourceFileLoader('local_gate', gate.__file__)
            module = importlib.util.module_from_spec(importlib.util.spec_from_loader('local_gate', local_loader))
            local_loader.exec_module(module)
        root = pathlib.Path(gate.__file__).resolve().parent
        self.assertEqual(pathlib.Path(module.SLOT), root / 'gate-slot')
        self.assertEqual(pathlib.Path(module.RUNNER), root / 'gate-slot-runner')

    def test_mutable_usr_directory_is_not_an_immutable_tool_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            with mock.patch.multiple(gate, TOOLCHAIN='/usr/bin', NEXTEST='/usr/bin', ORACLE='/bin/true'), \
                 mock.patch.object(gate, 'source_env_names', return_value=set()), \
                 mock.patch.object(gate, 'env', return_value={'RUSTC': '/usr/bin/false'}):
                self.assertEqual(gate.unpinned_executable(root, root), 'RUSTC')


class StressProtocol(unittest.TestCase):
    def test_native_iteration_identity_preserves_the_canonical_test_id(self):
        self.assertEqual(gate.canonical_test_name('pkg::bin@stress-59$module::test'),
                         'pkg::bin$module::test')
        self.assertEqual(gate.canonical_test_name('pkg::bin$module::test'),
                         'pkg::bin$module::test')

    def test_every_started_binary_must_finish_every_iteration(self):
        def suite(event, binary, index):
            return dict(type='suite', event=event, nextest=dict(
                crate='pkg', test_binary=binary, kind='test', stress_index=index, stress_total=2))
        events = [dict(type='test', event='ok'), suite('ok', 'a', 0), suite('ok', 'a', 1)]
        self.assertTrue(gate.stress_complete(events, 2))
        self.assertFalse(gate.stress_complete(events[:-1], 2))
        self.assertFalse(gate.stress_complete(events + [suite('started', 'b', 0)], 2))
        self.assertFalse(gate.stress_complete([dict(type='test', event='ok')], 2))
        self.assertTrue(gate.stress_complete([], 2), 'empty filtered suite is handled by nonempty gate')


class BuildAndRun(unittest.TestCase):
    def setUp(self):
        def source(*args, **kwargs):
            self.assertEqual(args[-1], 'source')
            self.assertTrue(args[-2].endswith('/gnu-tests/tools/gate_vacuity.py'))
            return b'{}'
        self.enterContext(mock.patch.object(gate, 'out', side_effect=source))

    def test_native_stress_uses_zero_retries_and_retains_all_iterations(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        calls = []
        def popen(argv, **kw):
            calls.append(argv)
            for index in range(2):
                kw['stdout'].write(json.dumps(dict(type='test', event='ok', name=f'pkg::bin@stress-{index}$probe')) + '\n')
                kw['stdout'].write(json.dumps(dict(type='suite', event='ok', nextest=dict(
                    crate='pkg', test_binary='bin', kind='test', stress_index=index, stress_total=2))) + '\n')
            kw['stderr'].write('Summary [fixture] 2/2 stress run iterations: 2 passed\n')
            return mock.Mock(wait=mock.Mock(return_value=0))
        with mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command,
                                 cargo_flags=lambda s: []), \
             mock.patch.object(gate.subprocess, 'call', listed_binaries), \
             mock.patch.object(gate.subprocess, 'Popen', popen):
            self.assertEqual(gate.build_and_run(root, root, 'test(probe)',
                {'vm2': 1, 'display': 1}, True, time.monotonic(), {}, repeat=2), [])
        self.assertEqual(len(calls), 2)
        for argv in calls:
            self.assertEqual(argv[argv.index('--stress-count') + 1], '2')
            self.assertEqual(argv[argv.index('--retries') + 1], '0')
            self.assertIn('--no-fail-fast', argv)

    def test_stress_zero_exit_cannot_hide_a_failed_body(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        def popen(argv, **kw):
            for index, event in enumerate(('ok', 'failed')):
                kw['stdout'].write(json.dumps(dict(type='test', event=event,
                    name=f'pkg::bin@stress-{index}$probe')) + '\n')
                kw['stdout'].write(json.dumps(dict(type='suite', event=event, nextest=dict(
                    crate='pkg', test_binary='bin', kind='test', stress_index=index, stress_total=2))) + '\n')
            kw['stderr'].write('Summary [fixture] 2/2 stress run iterations: 1 passed, 1 failed\n')
            return mock.Mock(wait=mock.Mock(return_value=0))
        with mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command,
                                 cargo_flags=lambda s: []), \
             mock.patch.object(gate.subprocess, 'call', listed_binaries), \
             mock.patch.object(gate.subprocess, 'Popen', popen):
            self.assertEqual(gate.build_and_run(root, root, 'test(probe)',
                {'vm2': 1, 'display': 1}, True, time.monotonic(), {}, repeat=2),
                ['pkg::bin$probe', 'pkg::bin$probe'])

    def run_protocol(self, records, repeat=1, ignored=(), selected=None, rc=0,
                     log='Summary [fixture] complete\n'):
        if selected is None:
            selected = {gate.canonical_test_name(r['name']).split('$', 1)[1]
                        for r in records if r.get('type') == 'test'} - set(ignored)
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        def listing(cmd, **kw):
            if '--binaries-metadata' in cmd:
                json.dump({'rust-suites': {'pkg': {'package-name': 'pkg', 'binary-name': 'bin',
                    'testcases': dict({name: {'ignored': True} for name in ignored}, **{
                        name: {'ignored': False, 'filter-match': {'status': 'matches'}}
                        for name in selected})}}}, kw['stdout'])
                return 0
            return listed_binaries(cmd, **kw)
        def popen(argv, **kw):
            for record in records:
                kw['stdout'].write(json.dumps(record) + '\n')
            kw['stderr'].write(log)
            return mock.Mock(wait=mock.Mock(return_value=rc))
        output = io.StringIO()
        with mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command,
                                 cargo_flags=lambda s: []), \
             mock.patch.object(gate.subprocess, 'call', listing), \
             mock.patch.object(gate.subprocess, 'Popen', popen), contextlib.redirect_stdout(output):
            result = gate.build_and_run(root, root, 'all()',
                {'vm2': 1, 'display': 1}, True, time.monotonic(), {}, repeat=repeat)
        self.assertEqual(list((root / 'ng').iterdir()), [], 'scratch left behind')
        return result, output.getvalue()

    def test_unfinished_test_is_red_and_named_in_normal_and_repeat_runs(self):
        for repeat in (1, 2):
            with self.subTest(repeat=repeat):
                records = [dict(type='test', event='started', name='pkg::bin$lost')]
                records += [dict(type='suite', event='ok', nextest=dict(crate='pkg',
                    test_binary='bin', kind='test', stress_index=i, stress_total=repeat))
                    for i in range(repeat)]
                result, output = self.run_protocol(records, repeat)
                self.assertIsNone(result, 'a started test without a result must refuse GREEN')
                self.assertIn('pkg::bin$lost', output)

    def test_ignored_start_without_result_is_not_a_failure(self):
        result, _ = self.run_protocol([
            dict(type='test', event='started', name='pkg::bin$ignored'),
            dict(type='test', event='ok', name='pkg::bin$passed')], ignored=('ignored',))
        self.assertEqual(result, [])

    def test_repeat_requires_each_test_result_even_when_all_binaries_finish(self):
        records = [dict(type='test', event='ok', name='pkg::bin@stress-0$short')]
        records += [dict(type='suite', event='ok', nextest=dict(crate='pkg',
            test_binary='bin', kind='test', stress_index=i, stress_total=2)) for i in range(2)]
        result, output = self.run_protocol(records, 2)
        self.assertIsNone(result, 'a test with fewer than N results must refuse GREEN')
        self.assertIn('pkg::bin$short', output)


    def test_missing_selected_body_is_incomplete_even_without_a_start(self):
        result, output = self.run_protocol([], selected={'absent'})
        self.assertIsNone(result)
        self.assertIn('pkg::bin$absent', output)

    def test_exit_summary_and_cancellation_must_agree_with_results(self):
        records = [dict(type='test', event='ok', name='pkg::bin$probe')]
        for rc, log in ((100, 'Summary [fixture] complete\n'),
                        (-15, 'Summary [fixture] complete\n'), (0, ''),
                        (0, 'Summary [fixture] complete\nCancelling due to signal\n'),
                        (0, 'Summary [fixture] complete\nwarning: 1 test not run\n')):
            with self.subTest(rc=rc, log=log):
                result, _ = self.run_protocol(records, rc=rc, log=log)
                self.assertIsNone(result)

    def test_tests_are_told_where_to_leave_notices_and_evidence_and_stale_ones_are_dropped(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        (root / gate.NOTICES).write_text('left over from an earlier run\n')
        (root / gate.EVIDENCE).write_text('parity-ratchet: PASS from an earlier run\n')
        envs = []
        def popen(argv, **kw):
            envs.append(kw['env'])
            kw['stdout'].write(json.dumps(dict(type='test', event='ok', name='pkg::bin$probe')) + '\n')
            kw['stderr'].write('Summary [fixture] complete\n')
            return mock.Mock(wait=mock.Mock(return_value=0))
        with mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command, cargo_flags=lambda s: []), \
             mock.patch.object(gate.subprocess, 'call', listed_binaries), \
             mock.patch.object(gate.subprocess, 'Popen', popen), contextlib.redirect_stdout(io.StringIO()):
            gate.build_and_run(root, root, 'all()', {'vm2': 1, 'display': 1}, True, time.monotonic(), {})
        self.assertTrue(envs)
        self.assertEqual({e.get('ROMACS_GATE_NOTICES') for e in envs}, {str(root / gate.NOTICES)})
        self.assertEqual({e.get('ROMACS_GATE_EVIDENCE') for e in envs}, {str(root / gate.EVIDENCE)})
        self.assertEqual({e.get('ROMACS_VACUITY_RECEIPT') for e in envs}, {str(root / 'VACUITY-VERDICT.json')})
        self.assertEqual({e.get('ROMACS_MEMBER_EVIDENCE') for e in envs}, {str(root / 'MEMBER-EVIDENCE.json')})
        self.assertFalse((root / gate.NOTICES).exists())
        self.assertFalse((root / gate.EVIDENCE).exists())

    def test_failed_and_timeout_outcomes_are_complete_but_not_green(self):
        for event in ('failed', 'timeout'):
            with self.subTest(event=event):
                result, _ = self.run_protocol([
                    dict(type='test', event=event, name='pkg::bin$probe')], rc=100)
                self.assertEqual(result, ['pkg::bin$probe', 'pkg::bin$probe'])

    def test_run_environment_creates_private_directories_and_drops_desktop_session(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        private = root / 'private'
        desktop = dict(DISPLAY=':97', WAYLAND_DISPLAY='wayland-fixture',
                       DBUS_SESSION_BUS_ADDRESS='unix:path=/private-bus')
        with mock.patch.dict(os.environ, desktop):
            effective = gate.run_env(root, private)
        for name in desktop:
            self.assertNotIn(name, effective)
        for name in ('HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME',
                     'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR', 'TMPDIR'):
            directory = pathlib.Path(effective[name])
            self.assertTrue(directory.is_relative_to(private))
            self.assertTrue(directory.is_dir(), name)
        self.assertEqual((private / 'run').stat().st_mode & 0o777, 0o700)
        self.assertEqual(effective['ROMACS_FORCE_ORACLE_PATH'], gate.ORACLE)
        self.assertEqual(effective['CARGO_WORKSPACE_DIR'], str(root))

    def test_run_environment_drops_a_running_editors_launch_variables(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        editor = dict(ROMACS_RUNTIME_ROOT='/installed/build', ROMACS_LISP_DIR='/installed/build/gnu-lisp',
                      ROMACS_USAGE_LOG='/sessions/ended/usage.jsonl', ROMACS_GUI_FONT='Iosevka',
                      ROMACS_GUI_FONT_SIZE='15')
        with mock.patch.dict(os.environ, editor):
            effective = gate.run_env(root, root / 'private')
        for name in editor:
            self.assertNotIn(name, effective)

    def test_font_cache_survives_private_home_normalization(self):
        for override in (None, '/persistent/fonts'):
            with mock.patch.dict(os.environ, {}, clear=True):
                if override is not None:
                    os.environ['ROMACS_TEST_FONTS_DIR'] = override
                actual = gate.test_env(pathlib.Path('/src'), pathlib.Path('/private'), names=set())
            self.assertEqual(actual['HOME'], '/private/home')
            self.assertEqual(actual['ROMACS_TEST_FONTS_DIR'],
                             override or str(gate.HOME / '.cache/romacs-test-fonts'))


    def test_exception_before_any_build_removes_scratch(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        with mock.patch.object(gate, 'SHORT', root / 'ng'), self.assertRaises(FileNotFoundError):
            gate.build_and_run(root / 'missing', root, 'all()', {}, True, time.monotonic(), {})
        self.assertEqual(list((root / 'ng').iterdir()), [], 'scratch left behind')

    def test_scratch_a_test_locked_is_removed(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        src = root / 'src'
        src.mkdir()
        def popen(argv, **kw):
            locked = pathlib.Path(kw['env']['TMPDIR']) / 'locked'
            (locked / 'inner').mkdir(parents=True)
            locked.chmod(0)
            kw['stderr'].write('Summary [fixture] 1 test run: 1 passed\n')
            return mock.Mock(wait=mock.Mock(return_value=0))
        with mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command, cargo_flags=lambda s: []), \
             mock.patch.object(gate.subprocess, 'call', listed_binaries), \
             mock.patch.object(gate.subprocess, 'Popen', popen), contextlib.redirect_stdout(io.StringIO()):
            gate.build_and_run(src, src, 'all()', {'vm2': 1, 'display': 1}, True, time.monotonic(), {})
        self.assertEqual(list((root / 'ng').iterdir()), [], 'scratch left behind')

    def test_exception_mid_run_removes_scratch_and_stops_tests(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        short, src, run_dir = root / 'ng', root / 'src', root / 'run'
        src.mkdir(), run_dir.mkdir()
        started, real_popen = [], subprocess.Popen

        def popen(cmd, **kw):
            if started:
                raise OSError('injected')
            started.append(real_popen(['sleep', '60'], **kw))
            return started[-1]

        with mock.patch.multiple(gate, SHORT=short, sh=metadata_command, cargo_flags=lambda s: [],
                                 ORACLE=str(root / 'oracle')), \
             mock.patch.object(gate.subprocess, 'call', listed_binaries), mock.patch.object(gate.subprocess, 'Popen', popen):
            with self.assertRaisesRegex(OSError, 'injected'):
                gate.build_and_run(src, run_dir, 'all()', {'vm2': 1, 'display': 1}, True, time.monotonic(), {})
        try:
            self.assertEqual(len(started), 1)
            self.assertIsNotNone(started[0].poll(), 'test process left running')
            self.assertEqual(list(short.iterdir()), [], 'scratch left behind')
        finally:
            if started[0].poll() is None:
                started[0].kill()
                started[0].wait()

    def test_gate_config_and_linux_video_apply_to_combined_build_and_reuse(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        calls = []
        def listing(argv, **kw):
            calls.append(argv)
            return listed_binaries(argv, **kw)
        def popen(argv, **kw):
            calls.append(argv)
            kw['stdout'].write('{"type":"test","event":"ok","name":"pkg::bin$probe"}\n')
            kw['stderr'].write('Summary [fixture] complete\n')
            return mock.Mock(wait=mock.Mock(return_value=0))
        for platform in ('linux', 'darwin'):
            calls.clear()
            with self.subTest(platform=platform), \
                 mock.patch.dict(FIXTURE_SUITES, {'display': ['romacs-display-runtime']}), \
                 mock.patch.object(gate.sys, 'platform', platform), \
                 mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command,
                                     cargo_flags=lambda s: []), \
                 mock.patch.object(gate.subprocess, 'call', listing), \
                 mock.patch.object(gate.subprocess, 'Popen', popen):
                self.assertEqual(gate.build_and_run(root, root, 'all()',
                    {'vm2': 1, 'display': 1}, True, time.monotonic(), {}), [])
            configs = [argv[argv.index('--config-file') + 1] for argv in calls]
            self.assertEqual(len(set(configs)), 1)
            config = pathlib.Path(configs[0]).read_text()
            self.assertNotIn('romacs-core', config)
            self.assertIn('slow-timeout', config)
            self.assertEqual('--features' in calls[0], platform == 'linux')
            if platform == 'linux':
                self.assertEqual(calls[0][calls[0].index('--features') + 1],
                                 'romacs-display-runtime/video')

    def test_combined_build_failure_starts_no_tests(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        with mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command, cargo_flags=lambda s: []), \
             mock.patch.object(gate.subprocess, 'call', return_value=1), \
             mock.patch.object(gate.subprocess, 'Popen') as runs:
            self.assertIsNone(gate.build_and_run(root, root, 'all()', {}, True, time.monotonic(), {}))
        runs.assert_not_called()
        self.assertEqual((root / 'failure-reason.txt').read_text().strip(),
                         'combined build failed (exit 1)')

    def test_inventory_failure_preserves_suite_and_exit(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        def listing(argv, **kw):
            if '--binaries-metadata' in argv:
                return 23
            return listed_binaries(argv, **kw)
        with mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command,
                                 cargo_flags=lambda s: []), \
             mock.patch.object(gate.subprocess, 'call', listing), \
             mock.patch.object(gate.subprocess, 'Popen') as runs:
            self.assertIsNone(gate.build_and_run(root, root, 'all()', {},
                                                True, time.monotonic(), {}))
        runs.assert_not_called()
        self.assertEqual((root / 'failure-reason.txt').read_text().strip(),
                         'vm2 test inventory failed (exit 23)')

    def test_missing_package_is_not_an_empty_green(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        def incomplete(cmd, **kw):
            json.dump({'rust-binaries': {}}, kw['stdout'])
            return 0
        with mock.patch.multiple(gate, SHORT=root / 'ng', sh=metadata_command, cargo_flags=lambda s: []), \
             mock.patch.object(gate.subprocess, 'call', incomplete), \
             mock.patch.object(gate.subprocess, 'Popen') as runs:
            with self.assertRaisesRegex(ValueError, 'missing binaries for vm2'):
                gate.build_and_run(root, root, 'all()', {}, True, time.monotonic(), {})
        runs.assert_not_called()


class OracleResolution(unittest.TestCase):
    def setUp(self):
        self.root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.oracle = pinned_fixture(self, self.root)

    def test_exact_private_pin_path_and_relative_root_normalization(self):
        self.assertEqual(gate._pinned_oracle(self.root), str(self.oracle))
        previous = pathlib.Path.cwd()
        try:
            os.chdir(self.root)
            with mock.patch.dict(os.environ, {'GNU_ORACLE_ROOT': 'oracle-root'}):
                self.assertEqual(gate._pinned_oracle(self.root), str(self.oracle))
        finally:
            os.chdir(previous)

    def test_missing_reference_refuses_with_restoration_action(self):
        (self.root / 'parity-reference.toml').unlink()
        with self.assertRaisesRegex(SystemExit, 'cannot read parity-reference.toml.*restore'):
            gate._pinned_oracle(self.root)

    def test_missing_or_malformed_commit_refuses_with_restoration_action(self):
        for text in ('other = 1\n', 'mirror_commit = "not-a-revision"\n'):
            (self.root / 'parity-reference.toml').write_text(text)
            with self.assertRaisesRegex(SystemExit, 'no valid mirror_commit.*restore'):
                gate._pinned_oracle(self.root)

    def test_missing_build_refuses_with_build_action(self):
        self.oracle.unlink()
        with self.assertRaisesRegex(SystemExit, 'no GNU oracle built.*tools/build-gnu-oracle.sh'):
            gate._pinned_oracle(self.root)

    def test_nonexecutable_build_refuses_with_build_action(self):
        self.oracle.chmod(0o644)
        with self.assertRaisesRegex(SystemExit, 'no GNU oracle built.*tools/build-gnu-oracle.sh'):
            gate._pinned_oracle(self.root)

    def test_reference_read_failure_is_actionable(self):
        with mock.patch.object(pathlib.Path, 'read_text', side_effect=PermissionError('denied')):
            with self.assertRaisesRegex(SystemExit, 'cannot read parity-reference.toml.*denied.*restore'):
                gate._pinned_oracle(self.root)

    def test_invalid_override_refuses_before_admission(self):
        with self.assertRaisesRegex(SystemExit, '--oracle must name an executable file'):
            gate._override_oracle(self.root / 'absent')
        self.oracle.chmod(0o644)
        with self.assertRaisesRegex(SystemExit, '--oracle must name an executable file'):
            gate._override_oracle(self.oracle)


class GateArguments(unittest.TestCase):

    def setUp(self):
        self.root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.enterContext(mock.patch.multiple(gate, HOME=self.root, CACHE=self.root / 'cache',
                                             SHORT=self.root / 'ng'))
        src = gate.CACHE / 'lanes/velocity-N4/src'
        src.mkdir(parents=True)
        (src / 'Cargo.toml').write_text('[profile.gate]\ninherits = "release"\n')
        self.oracle = pinned_fixture(self, self.root)
        (self.root / 'tools').mkdir()
        (self.root / 'tools/gate').write_bytes(pathlib.Path(gate.__file__).read_bytes())
        for source, name in ((gate.SLOT, 'gate-slot'), (gate.RUNNER, 'gate-slot-runner')):
            target = self.root / 'tools' / name
            target.write_bytes(pathlib.Path(source).read_bytes())
            target.chmod(0o755)
        self.enterContext(mock.patch.multiple(gate, SLOT=str(self.root / 'tools/gate-slot'),
                                             RUNNER=str(self.root / 'tools/gate-slot-runner')))
        subprocess.run(['git', 'init', '-b', 'master', str(self.root)], check=True, stdout=subprocess.DEVNULL)
        for key, value in [('user.name', 'Fixture'), ('user.email', 'fixture@example.invalid'), ('commit.gpgsign', 'false')]:
            subprocess.run(['git', 'config', key, value], cwd=self.root, check=True)
        subprocess.run(['git', 'add', '.'], cwd=self.root, check=True)
        subprocess.run(['git', 'commit', '-m', 'fixture'], cwd=self.root, check=True, stdout=subprocess.DEVNULL)
        previous = pathlib.Path.cwd()
        os.chdir(self.root)
        self.addCleanup(lambda: os.chdir(previous))

    def test_oracle_and_required_test_are_forwarded_to_frozen_admission(self):
        with mock.patch.object(gate, 'ORACLE', gate.ORACLE):
            cmd = self.slot_command(['--oracle', str(self.oracle), '--require', 'pkg::bin$probe'])
        payload = cmd[cmd.index('--') + 1:]
        self.assertEqual(payload[payload.index('--oracle') + 1], str(self.oracle))
        self.assertEqual(payload[payload.index('--require') + 1], 'pkg::bin$probe')
        self.assertEqual(payload[-1], 'focused')

    def test_staged_snapshot_excludes_other_work_without_changing_the_index(self):
        (self.root / 'captured-new').write_text('staged')
        subprocess.run(['git', 'add', 'captured-new'], cwd=self.root, check=True)
        (self.root / 'README.md').write_text('other work')
        (self.root / 'untracked').write_text('not staged')
        path = self.root / 'tools/gate'
        path.write_bytes(path.read_bytes() + b'\n# unstaged fixture\n')
        before = (self.root / '.git/index').read_bytes()
        staged = gate.freeze(self.root, staged=True)
        self.assertEqual((self.root / '.git/index').read_bytes(), before)
        names = gate.out('git', 'ls-tree', '-r', '--name-only', staged, cwd=self.root).decode().splitlines()
        self.assertIn('captured-new', names)
        self.assertNotIn('README.md', names)
        self.assertNotIn('untracked', names)
        self.assertNotIn(b'unstaged fixture', gate.out('git', 'show', staged + ':tools/gate', cwd=self.root))
        total = gate.freeze(self.root)
        self.assertEqual((self.root / '.git/index').read_bytes(), before)
        names = gate.out('git', 'ls-tree', '-r', '--name-only', total, cwd=self.root).decode().splitlines()
        self.assertIn('README.md', names)
        self.assertIn('untracked', names)
        self.assertIn(b'unstaged fixture', gate.out('git', 'show', total + ':tools/gate', cwd=self.root))

    def test_repeat_count_is_forwarded_and_invalid_counts_are_rejected(self):
        cmd = self.slot_command(['--repeat', '60'])
        payload = cmd[cmd.index('--') + 1:]
        self.assertEqual(payload[payload.index('--repeat') + 1], '60')
        for count in ('0', '-1'):
            with mock.patch.object(gate.sys, 'argv', ['gate', '--repeat', count, 'focused', 'all()']), \
                 self.assertRaises(SystemExit) as result:
                gate.gate_main()
            self.assertEqual(result.exception.code, 2)


    def slot_command(self, options):
        with mock.patch.object(gate.sys, 'argv', ['gate', '--lane', 'velocity-N4', *options,
                                                'focused', 'test(n4_selected)']), \
             mock.patch.object(gate.subprocess, 'call', return_value=23) as call:
            self.assertEqual(gate.main(), 23)
        call.assert_called_once()
        cmd = call.call_args.args[0]
        run_dir = pathlib.Path(cmd[0]).parent
        self.assertEqual(cmd[:2], [str(run_dir / 'gate-slot'), 'velocity-N4'])
        self.assertEqual((run_dir / 'gate-slot').read_bytes(), pathlib.Path(gate.SLOT).read_bytes())
        self.assertEqual((run_dir / 'gate-slot-runner').read_bytes(), pathlib.Path(gate.RUNNER).read_bytes())
        environment = call.call_args.kwargs['env']
        self.assertEqual(environment['ROMACS_SLOT_RUNNER'], str(run_dir / 'gate-slot-runner'))
        self.assertEqual(environment['ROMACS_SLOT'], cmd[0])
        return cmd

    def assert_nextest_threads(self, cmd, core, display):
        slot = cmd[:cmd.index('--')]
        self.assertEqual(slot[slot.index('--test-threads') + 1], str(core + display))
        payload = cmd[cmd.index('--') + 1:]
        run_dir = pathlib.Path(payload[payload.index('--run-dir') + 1])
        self.assertEqual(payload[:2], [gate.sys.executable, str(run_dir / 'gate')])
        runs = []

        def popen(argv, **kw):
            runs.append(argv)
            kw['stdout'].write('{"type":"test","event":"ok","name":"pkg::bin$probe"}\n')
            kw['stderr'].write('Summary [fixture] 1 test run: 1 passed, 0 skipped\n')
            return mock.Mock(wait=mock.Mock(return_value=0))

        def command(*args, **kwargs):
            if args[0] == 'tar':
                import tarfile
                with tarfile.open(fileobj=io.BytesIO(kwargs['input'])) as archive:
                    archive.extractall(args[3], filter='data')
            else:
                metadata_command(*args, **kwargs)
        archive = gate.out('git', 'archive', payload[payload.index('--tree') + 1], cwd=self.root)
        with mock.patch.object(gate.sys, 'argv', payload[1:]), \
             mock.patch.object(gate, 'sync'), mock.patch.object(gate, 'sh', side_effect=command), \
             mock.patch.object(gate, 'environment_identity', return_value=('fixture', {}, {})), \
             mock.patch.object(gate, 'out', return_value=archive), \
             mock.patch.object(gate.subprocess, 'call', side_effect=listed_binaries) as builds, \
             mock.patch.object(gate.subprocess, 'Popen', side_effect=popen):
            self.assertEqual(gate.main(), 0)
        self.assertEqual(len(runs), 2)
        resolutions = [call.args[0] for call in builds.call_args_list
                       if '--binaries-metadata' not in call.args[0]]
        inventories = [call.args[0] for call in builds.call_args_list
                       if '--binaries-metadata' in call.args[0]]
        self.assertEqual(len(resolutions), 1)
        self.assertEqual(len(inventories), 2)
        build = resolutions[0]
        packages = [build[i + 1] for i, arg in enumerate(build) if arg == '-p']
        self.assertEqual(packages, [p for suite in FIXTURE_SUITES.values() for p in suite])
        for suite, threads, argv in zip(('vm2', 'display'), (core, display), runs):
            with self.subTest(suite=suite):
                self.assertEqual(argv[argv.index('--test-threads') + 1], str(threads))
                self.assertEqual(argv[argv.index('-E') + 1], 'test(n4_selected)')
                self.assertEqual(pathlib.Path(argv[argv.index('--binaries-metadata') + 1]).name,
                                 f'{suite}-binaries.json')
                metadata = json.loads(pathlib.Path(argv[argv.index('--binaries-metadata') + 1]).read_text())
                self.assertEqual(set(metadata['rust-binaries']), set(FIXTURE_SUITES[suite]))
                self.assertEqual(metadata['rust-build-meta'], {'fixture': True})
        self.assertEqual(list(gate.SHORT.iterdir()), [])

    def test_reuse_resolves_the_oracle_in_the_outer_process(self):
        """reuse never spawns an inner run, so it must resolve the pin itself.

        Leaving ORACLE unset made `digest(None)` raise inside reuse(), which the
        gate reported as "receipt/input unavailable or malformed" -- and
        romacs-land.sh requires the exact string "gate: reuse GREEN", so every
        landing of every branch was refused.
        """
        with mock.patch.object(gate, 'ORACLE', None):
            captured = {}

            def fake_reuse(wt, args, group):
                captured['oracle'] = gate.ORACLE
                return 0

            with mock.patch.object(gate, 'reuse', fake_reuse), \
                 mock.patch.object(gate.sys, 'argv', ['gate', '--lane', 'x', 'reuse', 'run']):
                gate.gate_main()
        self.assertIsNotNone(
            captured['oracle'],
            'reuse ran with ORACLE unset; digest(None) would refuse it as malformed')
        self.assertEqual(captured['oracle'], str(self.oracle))

    def test_default_jobs_is_omitted_from_slot_argv(self):
        cmd = self.slot_command([])
        self.assertNotIn('--jobs', cmd[:cmd.index('--')])

    def test_default_threads_reach_inner_nextest(self):
        self.assert_nextest_threads(self.slot_command([]), 1, 1)

    def test_admission_keeps_the_frozen_runner_after_source_edits(self):
        cmd = self.slot_command([])
        pathlib.Path(gate.RUNNER).write_text('raise RuntimeError("edited after freeze")\n')
        result = subprocess.run([cmd[0], 'fixture', '--help', '--', 'true'],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('MemoryMax', result.stdout)

    def test_default_memory_limit_reaches_admission(self):
        cmd = self.slot_command([])
        self.assertEqual(cmd[cmd.index('--mem') + 1], '34G')

    def test_explicit_jobs_is_forwarded_unchanged(self):
        for jobs in (3, 8):
            with self.subTest(jobs=jobs):
                cmd = self.slot_command(['--jobs', str(jobs)])
                slot = cmd[:cmd.index('--')]
                self.assertEqual(slot.count('--jobs'), 1)
                self.assertEqual(slot[slot.index('--jobs') + 1], str(jobs))

    def test_explicit_threads_reach_inner_nextest_unchanged(self):
        for options, core, display in ((['--threads', '2'], 2, 1),
                                       (['--display-threads', '2'], 1, 2),
                                       (['--jobs', '3', '--threads', '2', '--display-threads', '1'], 2, 1)):
            with self.subTest(options=options):
                self.assert_nextest_threads(self.slot_command(options), core, display)


class Receipts(unittest.TestCase):
    """Real Git submissions and receipts; only the expensive Cargo/slot are stubbed."""
    run_dir: pathlib.Path

    def setUp(self):
        self.root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.wt = self.root / 'writer'
        self.wt.mkdir()
        self.git('init', '-b', 'master')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        (self.wt / 'tools').mkdir()
        (self.wt / 'tools/gate').write_bytes(pathlib.Path(gate.__file__).read_bytes())
        self.oracle = pinned_fixture(self, self.wt)
        (self.wt / 'f.rs').write_text('submitted')
        (self.wt / 'Cargo.toml').write_text('[profile.gate]\ninherits="release"\n')
        self.git('add', '.')
        self.git('commit', '-m', 'fixture')
        self.enterContext(mock.patch.multiple(gate, HOME=self.root, CACHE=self.root / 'cache',
                                             SHORT=self.root / 'ng'))
        self.enterContext(mock.patch.dict(os.environ, {'CARGO_HOME': str(self.root / 'cargo')}, clear=False))
        self.runner = self.root / 'runner'
        self.runner.write_text('runner')
        self.enterContext(mock.patch.multiple(gate, ORACLE=str(self.oracle), SLOT=str(self.runner), RUNNER=str(self.runner)))
        original = gate.out
        self.rustc = self.enterContext(mock.patch.object(gate, 'out', wraps=gate.out))
        self.rustc.side_effect = lambda *a, **k: b'rustc fixture' if a[:2] == ('rustc', '-vV') else original(*a, **k)
        self.after_submission = lambda: None
        self.slot_env = {}
        self.events = {'vm2': ['ok'], 'display': ['ok']}
        self.captured = []

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.wt, stderr=subprocess.DEVNULL)

    def invoke(self, *args):
        previous = pathlib.Path.cwd()
        os.chdir(self.wt)
        try:
            with mock.patch.object(gate.sys, 'argv', ['gate', *args]):
                return gate.main()
        finally:
            os.chdir(previous)

    def submit(self, mode='full', *options):
        def slot(cmd, env):
            self.after_submission()
            payload = cmd[cmd.index('--') + 1:]
            self.run_dir = pathlib.Path(payload[payload.index('--run-dir') + 1])
            self.assertEqual(pathlib.Path(payload[1]).read_bytes(), (self.wt / 'tools/gate').read_bytes())
            def admitted():
                with mock.patch.object(gate.sys, 'argv', payload[1:]), \
                     mock.patch.dict(os.environ, dict(env, **self.slot_env)), \
                     mock.patch.multiple(gate, SLOT=env['ROMACS_SLOT'], RUNNER=env['ROMACS_SLOT_RUNNER']):
                    return gate.main()
            if self.slot_env:


                child = multiprocessing.get_context('fork').Process(target=lambda: os._exit(admitted()))
                child.start()
                child.join(10)
                if child.is_alive():
                    child.kill()
                    child.join()
                    self.fail('admitted fixture did not finish')
                return child.exitcode
            return admitted()

        def build(src, run_dir, expr, threads, run, t0, suites, repeat=1):
            self.assertEqual(repeat, getattr(self, "repeat", 1))
            suites.update(FIXTURE_SUITES)
            self.captured.append((src / 'f.rs').read_text())
            if getattr(self, 'notice', None):
                (run_dir / gate.NOTICES).write_text(self.notice)
            failed = []
            if run:
                for suite, events in self.events.items():
                    (run_dir / f'{suite}.jsonl').write_text(''.join(
                        '{"type":"test","event":"%s","name":"selected"}\n' % event for event in events))
                    failed.extend('selected' for event in events if event in ('failed', 'timeout'))
            return failed

        with mock.patch.object(gate.subprocess, 'call', side_effect=slot), \
             mock.patch.object(gate, 'build_and_run', side_effect=build):
            rest = [options[0]] if mode == 'focused' else []
            flags = options[1:] if mode == 'focused' else options
            return self.invoke('--lane', 'writer', '--group', 'shared', *flags, mode, *rest)

    def reuse(self, *args):
        return self.invoke('--lane', 'reviewer', '--group', 'shared', 'reuse', str(self.run_dir), *args)

    def test_inner_receipt_reuses_from_caller_without_vacuity_git_root(self):
        # Real admission is a child: inner's exports must not leak to reuse.
        self.slot_env = {'HERMES_SESSION_ID': 'separate-admission'}
        os.environ.pop('ROMACS_VACUITY_GIT_ROOT', None)
        self.assertEqual(self.submit(), 0)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        revision = self.git('rev-parse', '--verify', 'HEAD').decode().strip()
        tree = self.git('rev-parse', '--verify', 'HEAD^{tree}').decode().strip()
        expected = __import__('hashlib').sha256(tree.encode()).hexdigest()
        for phase in ('build', 'test'):
            self.assertEqual(receipt['env_hashes'][phase].get('ROMACS_VACUITY_GIT_ROOT'),
                             expected, 'VACUITY_ROOT_REMAINS_FINGERPRINTED: ' + phase)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            result = self.reuse()
        self.assertEqual(result, 0, 'INNER_RECEIPT_REUSE_ACCEPTS: ' + output.getvalue())
        self.assertEqual(output.getvalue(), 'gate: reuse GREEN\n')
        # inner owns this input even when a caller supplies an unrelated root.
        with mock.patch.dict(os.environ, ROMACS_VACUITY_GIT_ROOT='/wrong-caller-root'):
            self.assertEqual(self.reuse(), 0, 'REUSE_RECONSTRUCTS_CANDIDATE_ROOT')

    def test_vacuity_root_same_tree_revision_reuses(self):
        self.slot_env = {'HERMES_SESSION_ID': 'separate-admission'}
        self.assertEqual(self.submit(), 0)
        self.git('commit', '--allow-empty', '-m', 'different revision, same tree')
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            result = self.reuse()
        self.assertEqual(result, 0, output.getvalue())
        self.assertEqual(output.getvalue(), 'gate: reuse GREEN\n')

    def test_staged_candidate_and_receipt_preserve_unrelated_work(self):
        (self.wt / 'f.rs').write_text('candidate')
        self.git('add', 'f.rs')
        (self.wt / 'README.md').write_text('other work')
        self.assertEqual(self.submit('full', '--staged'), 0)
        self.assertEqual(self.captured, ['candidate'])
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        names = self.git('ls-tree', '-r', '--name-only', receipt['tree']).decode().splitlines()
        self.assertNotIn('README.md', names)
        self.assertEqual(self.reuse('--staged'), 0)
        self.git('commit', '-m', 'candidate')
        self.assertEqual(self.reuse('--staged'), 0)
        self.assertEqual((self.wt / 'README.md').read_text(), 'other work')

    def test_vacuity_root_absent_or_invalid_refuses(self):
        with mock.patch.dict(os.environ, ROMACS_VACUITY_GIT_ROOT=str(self.wt)):
            valid = gate.environment_identity(self.wt, self.wt)[0]
        with mock.patch.dict(os.environ):
            os.environ.pop('ROMACS_VACUITY_GIT_ROOT', None)
            absent = gate.environment_identity(self.wt, self.wt)[0]
        self.assertNotEqual(valid, absent, 'ABSENT_ROOT_REFUSES')
        invalid = self.root / 'not-a-repository'
        invalid.mkdir()
        for root in (invalid, self.root / 'nonexistent'):
            with self.subTest(root=root), \
                 mock.patch.dict(os.environ, ROMACS_VACUITY_GIT_ROOT=str(root)):
                with self.assertRaises((OSError, subprocess.CalledProcessError),
                                       msg='INVALID_ROOT_REFUSES'):
                    gate.environment_identity(self.wt, self.wt)

    def test_inner_receipt_refuses_genuinely_missing_environment_input(self):
        self.slot_env = {'HERMES_SESSION_ID': 'separate-admission'}
        os.environ.pop('ROMACS_VACUITY_GIT_ROOT', None)
        with mock.patch.dict(os.environ, ROMACS_REUSE_REQUIRED_INPUT='recorded'):
            self.assertEqual(self.submit(), 0)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            result = self.reuse()
        self.assertEqual(result, 1, 'MISSING_SEMANTIC_INPUT_REFUSES: ' + output.getvalue())
        self.assertEqual(output.getvalue(),
                         'gate: reuse refused (environment: build missing: ROMACS_REUSE_REQUIRED_INPUT; '
                         'test missing: ROMACS_REUSE_REQUIRED_INPUT)\n',
                         'MISSING_SEMANTIC_INPUT_NAMED')

    def test_queued_pin_change_cannot_change_frozen_oracle(self):
        pin = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
        other = '1' * 40
        ref = self.wt / 'parity-reference.toml'
        ref.write_text(f'mirror_commit = "{pin}"\n')
        oracle_root = self.root / 'oracle-root'
        for revision in (pin, other):
            binary = oracle_root / revision / 'bin/emacs'
            binary.parent.mkdir(parents=True, exist_ok=True)
            binary.write_text('fixture ' + revision)
            binary.chmod(0o755)
        def queued():
            ref.write_text(f'mirror_commit = "{other}"\n')
            gate.ORACLE = None
        self.after_submission = queued
        with mock.patch.dict(os.environ, {'GNU_ORACLE_ROOT': str(oracle_root)}), \
             mock.patch.object(gate, 'ORACLE', None):
            self.assertEqual(self.submit(), 0)
            receipt = json.loads((self.run_dir / 'receipt.json').read_text())
            self.assertIn(pin, self.git('show', receipt['tree'] + ':parity-reference.toml').decode())
            self.assertEqual(receipt['oracle_sha'], gate.digest(oracle_root / pin / 'bin/emacs'),
                             'frozen tree A was certified with mutable worktree oracle B')

    def test_queued_reference_deletion_keeps_frozen_oracle(self):
        self.after_submission = lambda: (self.wt / 'parity-reference.toml').unlink()
        try:
            result = self.submit()
        except SystemExit as error:
            self.fail(f'queued reference deletion must not control frozen admission: {error}')
        self.assertEqual(result, 0)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        self.assertEqual(receipt['oracle_sha'], gate.digest(self.oracle))
        self.assertEqual(receipt['oracle_source'], 'pin')

    def test_override_is_recorded_and_cannot_reuse_as_pin_even_with_same_bytes(self):
        self.assertEqual(self.submit('full', '--oracle', str(self.oracle)), 0)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        self.assertEqual(receipt['oracle_source'], 'override', 'override must be explicit in receipt')
        self.assertEqual(receipt['oracle_sha'], gate.digest(self.oracle))
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(self.reuse(), 1, 'override receipt must not become pin-derived evidence')
        self.assertIn('oracle selection source', output.getvalue())
        self.assertEqual(self.reuse('--oracle', str(self.oracle)), 0)

    def test_explicit_override_wins_without_a_submitted_reference(self):
        (self.wt / 'parity-reference.toml').unlink()
        override = self.root / 'explicit-oracle'
        override.write_text('different diagnostic oracle')
        override.chmod(0o755)
        try:
            result = self.submit('full', '--oracle', str(override))
        except SystemExit as error:
            self.fail(f'explicit override must win over absent submitted pin: {error}')
        self.assertEqual(result, 0)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        self.assertEqual(receipt['oracle_sha'], gate.digest(override), 'explicit override did not win')
        self.assertEqual(receipt['oracle_source'], 'override')

    def test_default_resolution_reaches_real_reuse_without_inherited_oracle(self):
        self.assertEqual(self.submit(), 0)
        with mock.patch.object(gate, 'ORACLE', None):
            self.assertEqual(self.reuse(), 0, 'fresh default resolver must reach real reuse')
        self.oracle.write_text('changed oracle bytes')
        with mock.patch.object(gate, 'ORACLE', None):
            self.assertEqual(self.reuse(), 1, 'real reuse must reject changed oracle bytes')

    def test_copied_gate_in_fresh_interpreter_resolves_extracted_tree(self):
        self.assertEqual(self.submit(), 0)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        ref = self.wt / 'parity-reference.toml'
        ref.unlink()
        copied = self.run_dir / 'gate'
        help_run = subprocess.run([gate.sys.executable, str(copied), '--help'],
                                  cwd=self.root, capture_output=True, text=True)
        self.assertEqual(help_run.returncode, 0, 'copied gate failed fresh startup: ' + help_run.stderr)
        run_dir = self.root / 'fresh-run'
        runner = r'''
import json, pathlib, runpy, sys
ns = runpy.run_path(sys.argv[1], run_name='copied_gate')
g = ns['gate_main'].__globals__
root = pathlib.Path(sys.argv[2])
g.update(HOME=root, CACHE=root / 'cache', RUNNER=sys.argv[1])
g['environment_identity'] = lambda *a: ('fixture', {}, {})
def build(src, run_dir, expr, threads, run, t0, suites, repeat=1):
    assert not (src / '.git').exists(), 'test must execute a git-less mirror'
    assert (src / 'parity-reference.toml').is_file(), 'frozen reference missing from mirror'
    suites.update({'vm2': ['fixture']})
    (run_dir / 'vm2.jsonl').write_text('{"type":"test","event":"ok","name":"fixture"}\n')
    return []
g['build_and_run'] = build
sys.argv = [sys.argv[1], *sys.argv[3:]]
sys.exit(g['main']())
'''
        result = subprocess.run([gate.sys.executable, '-c', runner, str(copied), str(self.root),
            '--inner', '--worktree', str(self.wt), '--tree', receipt['tree'],
            '--run-dir', str(run_dir), '--group', 'fresh', 'full'],
            cwd=self.root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0,
                         'fresh copied inner did not certify frozen pin: ' + result.stdout + result.stderr)
        fresh = json.loads((run_dir / 'receipt.json').read_text())
        self.assertEqual(fresh['oracle_sha'], gate.digest(self.oracle))
        self.assertEqual(fresh['oracle_source'], 'pin')
        self.assertEqual(fresh['tree'], receipt['tree'])

    def test_empty_repeat_reports_no_tests_matched(self):
        self.repeat = 2
        self.events = {'vm2': [], 'display': []}
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(self.submit('focused', 'test(absent)', '--repeat', '2'), 1)
        self.assertIn('no tests matched the filter', output.getvalue())
        self.assertNotIn('a suite did not finish', output.getvalue())

    def test_a_notice_from_a_passing_test_is_printed_on_green_and_red(self):
        for events, rc, word in ((['ok'], 0, 'GREEN'), (['failed'], 1, 'RED')):
            with self.subTest(word=word):
                self.events = {'vm2': events, 'display': ['ok']}
                self.notice = 'parity-ratchet: UNCONFIRMED gnu:x: 1 floor member(s) lost; counted\n\n'
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    self.assertEqual(self.submit(), rc)
                self.assertIn(f'gate: {word}', output.getvalue())
                self.assertEqual([l for l in output.getvalue().splitlines() if 'NOTICE' in l],
                                 ['gate: NOTICE parity-ratchet: UNCONFIRMED gnu:x: 1 floor member(s) lost; counted'])

    def test_failed_body_receipt_is_complete_red_and_never_reusable(self):
        self.events = {'vm2': ['failed'], 'display': ['ok']}
        self.assertEqual(self.submit(), 1)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        self.assertTrue(receipt['complete'])
        self.assertEqual(receipt['verdict'], 'RED')
        self.assertEqual(receipt['outcomes']['vm2']['failed'], 1)
        self.assertEqual(self.reuse(), 1)

    def test_stress_receipt_is_never_promoted_to_reusable_evidence(self):
        self.assertEqual(self.submit(), 0)
        path = self.run_dir / 'receipt.json'
        receipt = json.loads(path.read_text())
        receipt['repeat'] = 60
        path.write_text(json.dumps(receipt))
        self.assertEqual(self.reuse(), 1)


    def test_slot_mirror_and_cross_slot_reuse(self):
        with mock.patch.dict(os.environ, {'NEOMACS_SLOT_INDEX': '0'}):
            self.assertEqual(self.submit(), 0)
        self.assertTrue((self.root / '.cache/neomacs-slots/0/src/f.rs').is_file())
        self.assertFalse((gate.CACHE / 'lanes/shared/src').exists())
        with mock.patch.dict(os.environ, {'NEOMACS_SLOT_INDEX': '2'}):
            self.assertEqual(self.reuse(), 0)
        self.assertEqual(self.reuse(), 0)

    def test_deleted_directory_symlink_does_not_survive_lane_switch(self):
        external = self.root / 'external'
        external.mkdir()
        (external / 'proof.txt').write_text('lane A data')
        link = self.wt / 'previous-lane'
        link.symlink_to(external, target_is_directory=True)
        old = self.wt / 'old-directory'
        old.mkdir()
        (old / 'data').write_text('old')
        self.git('add', '.')
        self.git('commit', '-m', 'lane A paths')
        with mock.patch.dict(os.environ, {'NEOMACS_SLOT_INDEX': '0'}):
            self.assertEqual(self.submit(), 0)
            dest = gate.mirror('shared')
            consumer = ['sh', '-c', 'test ! -e previous-lane/proof.txt && test ! -d old-directory']
            self.assertNotEqual(subprocess.run(consumer, cwd=dest).returncode, 0)
            (dest / 'untracked-leftover').write_text('not submitted')
            link.unlink()
            (old / 'data').unlink()
            old.rmdir()
            self.git('add', '-A')
            self.git('commit', '-m', 'lane B removes lane A paths')
            self.assertEqual(self.submit(), 0)
            cold = self.root / 'cold'
            archive = self.root / 'archive'
            archive.mkdir()
            gate.sh('tar', '-x', '-C', str(archive), input=self.git('archive', 'HEAD'))
            gate.sync(archive, cold)
            self.assertFalse(os.path.lexists(dest / 'previous-lane'))
            self.assertFalse((dest / 'old-directory').exists())
            self.assertFalse((dest / 'untracked-leftover').exists())
            self.assertEqual(subprocess.run(consumer, cwd=dest).returncode, 0)
            self.assertEqual(gate.mirror_files(dest), gate.mirror_files(cold))
            self.assertEqual((external / 'proof.txt').read_text(), 'lane A data')

    def test_mirror_file_list_mismatch_is_red_before_build(self):
        original = gate.sh
        def stale(*args, **kwargs):
            result = original(*args, **kwargs)
            if args[0] == 'rsync' and '--delete' in args:
                (pathlib.Path(args[-1]) / 'unexpected').write_text('stale')
            return result
        with mock.patch.object(gate, 'sh', side_effect=stale):
            self.assertEqual(self.submit(), 1)
        self.assertEqual(self.captured, [], 'must not build with a mismatched file list')

    def test_slot_index_is_not_an_ambient_build_control(self):
        with mock.patch.dict(os.environ, {'NEOMACS_SLOT_INDEX': '2'}):
            self.assertNotIn('NEOMACS_SLOT_INDEX', gate.env(names=set()))
            self.assertEqual(gate.env(names={'NEOMACS_SLOT_INDEX'})['NEOMACS_SLOT_INDEX'], '2')

    def test_scheduler_tool_paths_do_not_change_build_inputs(self):
        selectors = {'ROMACS_SLOT': '/run/one/slot', 'ROMACS_SLOT_RUNNER': '/run/one/runner'}
        with mock.patch.dict(os.environ, selectors):
            effective = gate.env(names=set())
            declared = gate.env(names=set(selectors))
        for key, value in selectors.items():
            self.assertNotIn(key, effective)
            self.assertEqual(declared[key], value)

    def test_source_declared_slot_index_still_refuses_cross_slot_reuse(self):
        (self.wt / 'f.rs').write_text('const INDEX: &str = env!("NEOMACS_SLOT_INDEX");')
        self.git('add', '.')
        self.git('commit', '-m', 'declare slot input')
        with mock.patch.dict(os.environ, {'NEOMACS_SLOT_INDEX': '0'}):
            self.assertEqual(self.submit(), 0)
            self.assertEqual(self.reuse(), 0)
        with mock.patch.dict(os.environ, {'NEOMACS_SLOT_INDEX': '2'}):
            self.assertEqual(self.reuse(), 1)

    def test_invalid_slot_index_refuses_before_sync(self):
        for index in ('../escape', '-1', '', '01'):
            with self.subTest(index=index), mock.patch.dict(os.environ, {'NEOMACS_SLOT_INDEX': index}):
                self.assertEqual(self.submit(), 1)

    def test_seed_is_refused_before_admission(self):
        for options in (('--seed', 'master'), ('--seed=master',), ('--seed',)):
            with self.subTest(options=options), mock.patch.object(gate.subprocess, 'call') as slot:
                with self.assertRaises(SystemExit) as error:
                    self.invoke(*options, 'full')
                self.assertEqual(error.exception.code, 2)
                slot.assert_not_called()


    def test_frozen_submission_keeps_index_and_queued_edits_out(self):
        before = self.git('ls-files', '--stage')
        (self.wt / 'untracked').write_text('included')
        self.after_submission = lambda: (self.wt / 'f.rs').write_text('queued edit')
        self.assertEqual(self.submit(), 0)
        self.assertEqual(self.captured, ['submitted'])
        self.assertEqual(self.git('ls-files', '--stage'), before)
        receipt = __import__('json').loads((self.run_dir / 'receipt.json').read_text())
        self.assertEqual(self.git('show', receipt['tree'] + ':untracked'), b'included')

    def test_deleted_source_is_absent_from_frozen_submission_and_mirror(self):
        self.assertEqual(self.submit(), 0)
        path = self.wt / 'deleted.rs'
        path.write_text('obsolete source')
        self.git('add', '.')
        self.git('commit', '-m', 'deletion fixture')
        self.assertEqual(self.submit(), 0)
        self.assertTrue((gate.mirror('shared') / path.name).exists())
        path.unlink()
        before = (self.wt / '.git/index').read_bytes()
        self.assertEqual(self.submit(), 0)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        self.assertNotIn(b'deleted.rs', self.git('ls-tree', '-r', '--name-only', receipt['tree']).splitlines())
        self.assertFalse((gate.mirror('shared') / path.name).exists())
        self.assertEqual((self.wt / '.git/index').read_bytes(), before)

    def test_force_added_ignored_submission_preserves_file_and_caller_index(self):
        (self.wt / '.gitignore').write_text('*.ignored\n')
        self.git('add', '.gitignore')
        self.git('commit', '-m', 'ignore fixture outputs')
        included = self.wt / 'included.ignored'
        included.write_text('explicitly staged')
        (self.wt / 'excluded.ignored').write_text('not submitted')
        self.git('add', '-f', 'included.ignored')
        included.write_text('submitted working content')
        (self.wt / 'f.rs').write_text('unstaged source')
        index = self.wt / '.git/index'
        before = index.read_bytes()
        staged = self.git('ls-files', '--stage', '-z')
        self.assertEqual(self.submit(), 0)
        self.assertEqual(index.read_bytes(), before, 'freeze changed the caller index bytes')
        self.assertEqual(self.git('ls-files', '--stage', '-z'), staged)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        tree = receipt['tree']
        paths = self.git('ls-tree', '-r', '--name-only', '-z', tree).split(b'\0')
        self.assertIn(b'included.ignored', paths,
                      'freeze silently dropped a force-added ignored staged file')
        self.assertNotIn(b'excluded.ignored', paths)
        self.assertEqual(self.git('show', tree + ':included.ignored'), b'submitted working content')
        self.assertEqual((gate.mirror('shared') / 'included.ignored').read_bytes(),
                         b'submitted working content')
        self.assertEqual(self.captured, ['unstaged source'])

    def test_zero_and_required_ignored_are_red(self):
        for events, options in (({'vm2': [], 'display': []}, ()),
                                ({'vm2': ['ok'], 'display': []}, ()),
                                ({'vm2': ['ignored'], 'display': ['ok']}, ('--require', 'absent'))):
            with self.subTest(events=events):
                self.events = events
                self.assertEqual(self.submit('full', *options), 1)

    def test_focused_zero_tests_are_red_but_one_suite_is_enough(self):
        for events, expected in (({'vm2': [], 'display': []}, 1),
                                 ({'vm2': ['ok'], 'display': []}, 0)):
            with self.subTest(events=events):
                self.events = events
                self.assertEqual(self.submit('focused', 'test(selected)'), expected)
                receipt = json.loads((self.run_dir / 'receipt.json').read_text())
                self.assertEqual(receipt['verdict'], 'RED' if expected else 'GREEN')
                self.assertEqual(receipt['selected'], {suite: len(selected) for suite, selected in events.items()})

    def test_writer_receipt_reused_by_clean_reviewer_and_inputs_refuse(self):
        self.assertEqual(self.submit(), 0)
        reviewer = self.root / 'reviewer'
        self.git('worktree', 'add', '--detach', str(reviewer), 'HEAD')
        self.wt = reviewer
        self.assertEqual(self.reuse('focused', 'test(selected)', '--require', 'selected'), 0)
        for key, value in [('RUSTFLAGS', '-C opt-level=1'), ('CARGO_FOO', '1')]:
            with self.subTest(key=key), mock.patch.dict(os.environ, {key: value}):
                self.assertEqual(self.reuse(), 1)
        with mock.patch.dict(os.environ, {'CARGO_TARGET_DIR': '/unused', 'CARGO_BUILD_JOBS': '17',
                                          'NEXTEST_TEST_THREADS': '7', 'RUST_TEST_THREADS': '3',
                                          'CARGO_INCREMENTAL': '0', 'SHLVL': '17', '_': 'other-command',
                                          'TERM': 'dumb', 'COLORTERM': 'truecolor'}):
            self.assertEqual(self.reuse(), 0)
        with mock.patch.dict(os.environ, {'CARGO_PROFILE_GATE_LTO': 'fat'}):
            self.assertEqual(self.reuse(), 0)
            self.assertNotIn('CARGO_PROFILE_GATE_LTO', gate.env())
        config = self.root / 'cargo/config.toml'
        config.parent.mkdir()
        config.write_text('[env]\nINPUT={value="fixture",relative=true}\n')
        self.assertEqual(self.reuse(), 1)
        self.assertEqual(self.submit(), 0)
        self.assertEqual(self.reuse(), 0)
        config.write_text('[env]\nINPUT="changed"\n')
        self.assertEqual(self.reuse(), 1)
        config.write_text('[env]\nINPUT={value="fixture",relative=true}\n')
        other = self.root / 'other/config.toml'
        other.parent.mkdir()
        other.write_bytes(config.read_bytes())
        with mock.patch.dict(os.environ, {'CARGO_HOME': str(other.parent)}):
            self.assertEqual(self.reuse(), 1)
        old = self.rustc.side_effect
        self.rustc.side_effect = lambda *a, **k: b'rustc changed' if a[:2] == ('rustc', '-vV') else old(*a, **k)
        self.assertEqual(self.reuse(), 1)
        self.rustc.side_effect = old

    def test_shell_profile_environment_reuses_same_tools(self):


        self.slot_env = dict(BAT_THEME='writer', BROWSER='writer-browser',
                             MANPAGER='writer-pager', MANROFFOPT='writer-options',
                             MISE_SHELL='writer-shell', PYTHONUNBUFFERED='1',
                             SUDO_EDITOR='writer-editor', PATH=os.environ['PATH'] + ':/unused')
        self.assertEqual(self.submit(), 0)
        with mock.patch.dict(os.environ, {'BAT_THEME': 'reviewer', 'BROWSER': 'reviewer-browser',
                                          'MANPAGER': 'reviewer-pager', 'PATH': '/unused:' + os.environ['PATH']}):
            self.assertEqual(self.reuse(), 0)
        self.assertNotIn('BROWSER', gate.test_env(self.wt, self.root / 'test-home'))

    def test_source_declared_environment_refuses_and_names_differences(self):
        import contextlib, io, json
        (self.wt / 'build.rs').write_text('fn main() { println!("cargo:rerun-if-env-changed=BUILD_INPUT"); }')
        (self.wt / 'f.rs').write_text('const A: &str = env!("MACRO_INPUT");\n'
                                      'const B: Option<&str> = option_env!("OPTION_INPUT");\n')
        self.git('add', '.')
        self.git('commit', '-m', 'environment inputs')
        self.assertEqual(self.submit(), 0)
        for name in ('BUILD_INPUT', 'MACRO_INPUT', 'OPTION_INPUT'):
            output = io.StringIO()
            with mock.patch.dict(os.environ, {name: 'private-value'}), contextlib.redirect_stdout(output):
                self.assertEqual(self.reuse(), 1)
            self.assertIn(name, output.getvalue())
            self.assertNotIn('private-value', output.getvalue())
        with mock.patch.dict(os.environ, {'BUILD_INPUT': 'writer-value'}):
            self.assertEqual(self.submit(), 0)
        receipt = json.loads((self.run_dir / 'receipt.json').read_text())
        self.assertEqual(receipt['env_hashes']['build']['BUILD_INPUT'],
                         __import__('hashlib').sha256(b'writer-value').hexdigest())
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(self.reuse(), 1)
        self.assertIn('BUILD_INPUT', output.getvalue())
        self.assertIn('missing:', output.getvalue())
        output = io.StringIO()
        with mock.patch.dict(os.environ, {'BUILD_INPUT': 'changed-value'}), contextlib.redirect_stdout(output):
            self.assertEqual(self.reuse(), 1)
        self.assertIn('changed: BUILD_INPUT', output.getvalue())
        self.assertNotIn('writer-value', json.dumps(receipt))

    def test_different_resolved_rustc_refuses_and_names_tool(self):
        import contextlib, io
        self.assertEqual(self.submit(), 0)
        tools = self.root / 'different-tools'
        tools.mkdir()
        rustc = tools / 'rustc'
        rustc.write_text('#!/bin/sh\nprintf "different compiler\\n"\n')
        rustc.chmod(0o755)
        output = io.StringIO()
        with mock.patch.dict(os.environ, {'RUSTC': str(rustc)}), \
             contextlib.redirect_stdout(output):
            self.assertEqual(self.reuse(), 1)
        self.assertIn('RUSTC', output.getvalue())
        with mock.patch.dict(os.environ, {'RUSTC': str(rustc)}):
            self.assertEqual(self.submit(), 0)
            rustc.write_text('#!/bin/sh\nprintf "changed compiler\\n"\n')
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(self.reuse(), 1)
            self.assertIn('RUSTC', output.getvalue())

    def test_review4_override_sh_cannot_enter_gate_path(self):
        tools = self.root / 'review4-toolchain'
        tools.mkdir()
        for name in ('cargo', 'rustc'):
            (tools / name).symlink_to(pathlib.Path(gate.TOOLCHAIN) / name)
        shell = tools / 'sh'
        shell.write_text('#!/bin/sh\nexec /bin/sh "$@"\n')
        shell.chmod(0o755)
        with mock.patch.dict(os.environ, {'ROMACS_GATE_TOOLCHAIN': str(tools)}):
            before = gate.environment_identity(self.wt)
            shell.write_text('#!/bin/sh\nexit 17\n')
            self.assertEqual(gate.environment_identity(self.wt), before)
            self.assertEqual(subprocess.run(['sh', '-c', 'exit 0'], env=gate.env()).returncode, 0)
            self.assertNotIn(str(tools), gate.gate_path())

    def test_review4_cargo_env_selectors_refuse_same_path_reuse(self):
        import contextlib, io
        config = self.root / 'cargo/config.toml'
        config.parent.mkdir()
        tool = self.root / 'pkg-helper'
        for name, value in (('PKG_CONFIG', str(tool)), ('RUSTFLAGS', f'-C linker={tool}')):
            with self.subTest(name=name):
                config.write_text(f'[env]\n{name}={{value="{value}",force=true}}\n')
                tool.write_text('#!/bin/sh\nexit 0\n')
                tool.chmod(0o755)
                self.assertEqual(self.submit(), 0)
                tool.write_text('#!/bin/sh\nexit 17\n')
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    self.assertEqual(self.reuse(), 1)
                self.assertIn(f'unpinned executable: [env] {name}', output.getvalue())

    def test_unpinned_selector_routes_and_fixed_positives(self):
        tool = self.root / 'external-tool'
        tool.write_text('#!/bin/sh\nexit 0\n')
        tool.chmod(0o755)
        for name in ('CC', 'CXX', 'AR', 'PKG_CONFIG', 'RUSTC', 'RUSTC_WORKSPACE_WRAPPER',
                     'CARGO_BUILD_RUSTC_WRAPPER', 'CARGO_TARGET_EXAMPLE_LINKER',
                     'CARGO_TARGET_EXAMPLE_RUNNER'):
            with self.subTest(name=name), mock.patch.dict(os.environ, {name: str(tool)}):
                self.assertEqual(gate.unpinned_executable(self.wt, self.wt), name)

        with mock.patch.dict(os.environ, {'RUSTC_WRAPPER': str(tool)}):
            self.assertIsNone(gate.unpinned_executable(self.wt, self.wt))
        for name, value in (('CC', '/usr/bin/cc'), ('AR', 'ar'),
                            ('RUSTC', str(pathlib.Path(gate.TOOLCHAIN) / 'rustc')),
                            ('PKG_CONFIG', 'pkg-config')):
            with self.subTest(name=name), mock.patch.dict(os.environ, {name: value}):
                self.assertIsNone(gate.unpinned_executable(self.wt, self.wt))
        config = self.root / 'cargo/config.toml'
        config.parent.mkdir()
        for setting, label in (
                (f'[env]\nRUSTC_WRAPPER="{tool}"\n', '[env] RUSTC_WRAPPER'),
                ('[env]\nCC={value="external-tool",relative=true}\n', '[env] CC'),
                (f'[target.example]\nrunner=["{tool}","arg"]\n', '[target.example] runner'),
                (f'[target.example]\nrustflags=["-C","linker={tool}"]\n', '[target.example] rustflags'),
                (f'[build]\nrustc="{tool}"\n', '[build] rustc'),
                (f'[build]\nrustc-wrapper="{tool}"\n', '[build] rustc-wrapper'),
                (f'[build]\nrustflags="-Clinker={tool}"\n', '[build] rustflags'),
                (f'[env]\nCARGO_ENCODED_RUSTFLAGS="-C\\u001flinker={tool}"\n', '[env] CARGO_ENCODED_RUSTFLAGS'),
                (f'[target.example]\nrunner=["/bin/sh","{tool}"]\n', '[target.example] runner')):
            with self.subTest(label=label):
                config.write_text(setting)
                self.assertEqual(gate.unpinned_executable(self.wt, self.wt), label)
        config.write_text('[env]\nRUSTC_WRAPPER="' + gate.SCCACHE + '"\n')
        self.assertIsNone(gate.unpinned_executable(self.wt, self.wt))

    def test_normal_pinned_reuse_is_green(self):
        self.assertEqual(self.submit(), 0)
        self.assertEqual(self.reuse(), 0)

    def test_shadowed_cat_cannot_change_test_outcome(self):
        tools = self.root / 'shadow'
        tools.mkdir()
        cat = tools / 'cat'
        cat.write_text('#!/bin/sh\nexit 17\n')
        cat.chmod(0o755)
        before = gate.environment_identity(self.wt)
        with mock.patch.dict(os.environ, {'PATH': str(tools) + ':' + os.environ['PATH']}):
            effective = gate.test_env(self.wt, self.root / 'home')
            self.assertEqual(subprocess.run(['cat'], input=b'', env=effective).returncode, 0)
            self.assertEqual(gate.environment_identity(self.wt), before)

    def test_bare_background_path_resolves_gate_toolchain(self):
        with mock.patch.dict(os.environ, {'PATH': '/usr/bin:/bin'}):
            effective = gate.env(self.wt)
            for tool in ('cargo', 'rustc', 'cargo-nextest'):
                self.assertIsNotNone(shutil.which(tool, path=effective['PATH']), tool)
            gate.environment_identity(self.wt)

    def test_target_native_tool_refuses_reuse(self):
        tool = self.root / 'native-tool'

        for name in ('TARGET_CC', 'TARGET_AR', 'CC_x86_64_unknown_linux_gnu',
                     'CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER'):
            with self.subTest(name=name), mock.patch.dict(os.environ, {name: str(tool)}):
                tool.write_text('#!/bin/sh\nexit 0\n')
                tool.chmod(0o755)
                self.assertIsNotNone(gate.unpinned_executable(self.wt, self.wt))
                tool.write_text('#!/bin/sh\nexit 17\n')
                self.assertIsNotNone(gate.unpinned_executable(self.wt, self.wt))

    def test_missing_pinned_tool_is_clean_red(self):
        import contextlib, io
        output = io.StringIO()
        with mock.patch.object(gate, 'TOOLCHAIN', str(self.root / 'missing')), \
             contextlib.redirect_stdout(output):
            self.assertEqual(self.submit(), 1)
        self.assertIn('gate: RED', output.getvalue())
        self.assertIn('cargo', output.getvalue())
        self.assertNotIn('Traceback', output.getvalue())

    def test_configured_tool_refuses_reuse(self):
        config = self.root / 'cargo/config.toml'
        config.parent.mkdir()
        tool = self.root / 'configured-tool'
        for template in ('[target.x86_64-unknown-linux-gnu]\nlinker="{tool}"\n',
                         '[build]\nrustc-workspace-wrapper="{tool}"\n',
                         '[env]\nTARGET_CC="{tool}"\n',
                         '[env]\nTARGET_AR={{value="{tool}",force=true}}\n'):
            with self.subTest(template=template):
                config.write_text(template.format(tool=tool))
                tool.write_text('#!/bin/sh\nexit 0\n')
                tool.chmod(0o755)
                self.assertIsNotNone(gate.unpinned_executable(self.wt, self.wt))
                tool.write_text('#!/bin/sh\nexit 17\n')
                self.assertIsNotNone(gate.unpinned_executable(self.wt, self.wt))

    def test_cargo_configuration_cannot_override_fixed_path(self):
        config = self.root / 'cargo/config.toml'
        config.parent.mkdir()
        config.write_text('[env]\nPATH={value="/shadow",force=true}\n')
        self.assertEqual(gate.unpinned_executable(self.wt, self.wt), '[env] PATH')

    def test_rustflags_selected_linker_refuses_reuse(self):
        tool = self.root / 'flag-linker'
        for name, flags in (('RUSTFLAGS', f'-C linker={tool}'),
                            ('RUSTFLAGS', f'-Clinker={tool}'),
                            ('CARGO_ENCODED_RUSTFLAGS', f'-C\x1flinker={tool}'),
                            ('RUSTFLAGS', f'--codegen=linker={tool}'),
                            ('RUSTFLAGS', f'-C link-arg=-fuse-ld={tool}'),
                            ('RUSTFLAGS', '-C opt-level=1')):
            with self.subTest(flags=flags), mock.patch.dict(os.environ, {name: flags}):
                tool.write_text('#!/bin/sh\nexit 0\n')
                tool.chmod(0o755)
                self.assertIsNotNone(gate.unpinned_executable(self.wt, self.wt))
                tool.write_text('#!/bin/sh\nexit 17\n')
                self.assertIsNotNone(gate.unpinned_executable(self.wt, self.wt))

    def test_fixed_path_is_fingerprinted_and_overrides_are_ignored(self):
        identity = gate.environment_identity(self.wt)
        expected = __import__('hashlib').sha256(gate.env(self.wt)['PATH'].encode()).hexdigest()
        self.assertEqual(identity[1]['build']['PATH'], expected)
        self.assertEqual(identity[1]['test']['PATH'], expected)
        for value in ('', 'relative/bin', '/somewhere:/shadow'):
            with self.subTest(value=value), mock.patch.dict(os.environ, {
                    'ROMACS_GATE_TOOLCHAIN': value, 'ROMACS_GATE_NEXTEST': value}):
                self.assertEqual(gate.env()['PATH'], gate.gate_path())
                self.assertEqual(gate.gate_path().split(':')[:2], [gate.TOOLCHAIN, gate.NEXTEST])

    def test_environment_source_scan_failure_refuses(self):
        self.assertEqual(self.submit(), 0)

        with mock.patch.object(gate, 'source_env_names', side_effect=OSError('private-error')):
            self.assertEqual(self.reuse(), 1)

    def test_discovered_native_tool_templates_and_unsupported_scan(self):
        (self.wt / 'build.rs').write_text('''
fn tool(variable: &str) {
    for name in [format!("{variable}_{target}"), format!("TARGET_{variable}"), variable.to_owned()] {
        println!("cargo:rerun-if-env-changed={name}");
    }
}
fn main() { tool("CUSTOM_TOOL"); }
''')
        with mock.patch.dict(os.environ, {'CUSTOM_TOOL_target': 'private-value', 'TARGET_CUSTOM_TOOL': 'other-value'}):
            names = gate.source_env_names(self.wt)
            self.assertTrue({'CUSTOM_TOOL', 'CUSTOM_TOOL_target', 'TARGET_CUSTOM_TOOL'} <= names)
            effective = gate.env(self.wt)
            self.assertEqual(effective['CUSTOM_TOOL_target'], 'private-value')
        for source in ('const X: &str = env!(concat!("INPUT", "_NAME"));',
                       'fn main() { println!("cargo:rerun-if-env-changed={arbitrary}"); }'):
            (self.wt / 'build.rs').write_text(source)
            with self.assertRaises(ValueError):
                gate.source_env_names(self.wt)

    def test_undeclared_shell_control_cannot_reach_commands(self):
        self.assertEqual(self.submit(), 0)
        with mock.patch.dict(os.environ, {'SHELL_PROFILE_UNKNOWN': 'different'}):
            self.assertEqual(self.reuse(), 0)
            self.assertNotIn('SHELL_PROFILE_UNKNOWN', gate.env(self.wt))
            self.assertNotIn('SHELL_PROFILE_UNKNOWN', gate.test_env(self.wt, self.root / 'home'))

    def test_reuse_refusal_names_input_class_without_values(self):
        import contextlib, io
        self.assertEqual(self.submit(), 0)
        output = io.StringIO()
        with mock.patch.dict(os.environ, {'ROMACS_MODE_LINE_FLOW': 'private-value'}), \
             contextlib.redirect_stdout(output):
            self.assertEqual(self.reuse(), 1)
        self.assertIn('environment', output.getvalue())
        self.assertNotIn('private-value', output.getvalue())

    def test_cross_session_receipt_reuse(self):
        with mock.patch.dict(os.environ, {'HERMES_SESSION_ID': 'writer', 'HERMES_UI_SESSION_ID': 'writer-ui'}):
            self.assertEqual(self.submit(), 0)
            for key in ('HERMES_SESSION_ID', 'HERMES_UI_SESSION_ID'):
                with self.subTest(key=key), mock.patch.dict(os.environ, {key: 'reviewer'}):
                    self.assertEqual(self.reuse(), 0)

    def test_invocation_noise_deny_list(self):
        self.assertEqual(self.submit(), 0)
        names = ('HERMES_NEW_METADATA', 'AI_AGENT', 'TERM_PROGRAM', 'COLORTERM',
                 'SSH_AUTH_SOCK', 'XDG_SESSION_ID', 'XDG_VTNR', 'WINDOWID',
                 'INVOCATION_ID', 'JOURNAL_STREAM', 'SYSTEMD_EXEC_PID', 'MANAGERPID',
                 'PWD', 'OLDPWD', 'SHLVL', '_', 'GIT_PAGER', 'PAGER', 'LESSCHARSET',
                 '__MISE_SESSION', '__MISE_DIFF')
        for name in names:
            with self.subTest(name=name), mock.patch.dict(os.environ, {name: 'other-invocation'}):
                self.assertEqual(self.reuse(), 0)

    def test_overridden_controls_match_effective_environment(self):
        self.assertEqual(self.submit(), 0)
        for name in ('CARGO_PROFILE_GATE_LTO',
                     'RUSTUP_TOOLCHAIN', 'NEXTEST_EXPERIMENTAL_LIBTEST_JSON',
                     'RUSTC_WRAPPER', 'SCCACHE_DIR', 'SCCACHE_LOG', 'SCCACHE_ERROR_LOG',
                     'SCCACHE_CONF', 'SCCACHE_CLIENT_SIDE', 'SCCACHE_CACHE_SIZE'):
            with self.subTest(name=name), mock.patch.dict(os.environ, {name: 'ignored-override'}):
                self.assertEqual(self.reuse(), 0)
                effective = gate.env()
                self.assertNotEqual(effective.get(name), 'ignored-override')

    def test_outer_environment_reconstructs_slot_environment(self):
        self.slot_env = dict(CARGO_PROFILE_DEV_DEBUG='line-tables-only',
                             CARGO_PROFILE_TEST_DEBUG='line-tables-only', CARGO_INCREMENTAL='0',
                             RUSTC_WRAPPER=shutil.which('sccache') or str(gate.HOME / '.nix-profile/bin/sccache'),
                             SCCACHE_CLIENT_SIDE='1', SCCACHE_DIR=str(gate.HOME / '.cache/sccache'),
                             SCCACHE_CACHE_SIZE='20G', SCCACHE_CONF='/dev/null',
                             INVOCATION_ID='admitted', CARGO_BUILD_JOBS='3',
                             CARGO_TARGET_DIR='/slot-target', NEXTEST_TEST_THREADS='12', RUST_TEST_THREADS='12')
        self.assertEqual(self.submit(), 0)
        self.assertEqual(self.reuse(), 0)
        with mock.patch.dict(os.environ, {'ROMACS_MODE_LINE_FLOW': 'off'}):
            self.assertEqual(self.reuse(), 1)

    def test_semantic_runtime_knob_change_refuses_reuse(self):
        with mock.patch.dict(os.environ, {'ROMACS_MODE_LINE_FLOW': '1'}):
            self.assertEqual(self.submit(), 0)
            self.assertEqual(self.reuse(), 0)
        with mock.patch.dict(os.environ, {'ROMACS_MODE_LINE_FLOW': 'off'}):
            effective = gate.run_env(self.wt, self.root / 'runtime-home')
            self.assertEqual(effective['ROMACS_MODE_LINE_FLOW'], 'off')
            self.assertEqual(self.reuse(), 1)

    def test_unknown_environment_changes_refuse_reuse(self):
        (self.wt / 'f.rs').write_text('const CONTROL: Option<&str> = option_env!("UNKNOWN_RUNTIME_CONTROL");')
        self.git('add', '.')
        self.git('commit', '-m', 'declared runtime input')
        for key in ('UNKNOWN_RUNTIME_CONTROL', 'ROMACS_UNKNOWN_VM_CONTROL', 'ROMACS_UNKNOWN_EDITOR_CONTROL',
                    'CARGO_UNKNOWN_CONTROL', 'RUST_UNKNOWN_CONTROL', 'LANG', 'LC_ALL', 'TZ'):
            with self.subTest(key=key), mock.patch.dict(os.environ, {key: 'original'}):
                self.assertEqual(self.submit(), 0)
                self.assertEqual(self.reuse(), 0)
                with mock.patch.dict(os.environ, {key: 'changed'}):
                    self.assertEqual(self.reuse(), 1)
                del os.environ[key]
                self.assertEqual(self.reuse(), 1)


    def test_config_origins_in_cargo_lookup_order(self):
        left, right = gate.CACHE / 'lanes/left/src', gate.CACHE / 'lanes/right/src'
        for src in (left, right):
            src.mkdir(parents=True)
            config = src.parent / '.cargo/config.toml'
            config.parent.mkdir(parents=True)
            config.write_text('[env]\nINPUT={value="fixture",relative=true}\n')
        self.assertEqual(gate.env_fingerprint(left), gate.env_fingerprint(left))
        self.assertNotEqual(gate.env_fingerprint(left), gate.env_fingerprint(right))


    def test_focused_coverage_is_exact_and_build_never_reuses(self):
        def submit_focused(*args):
            return self.submit('focused', 'test(selected)')
        self.assertEqual(submit_focused(), 0)
        self.assertEqual(self.reuse('focused', 'test(selected)'), 0)
        self.assertEqual(self.reuse('focused', 'test(other)'), 1)
        self.assertEqual(self.reuse(), 1)
        self.assertEqual(self.submit('build'), 0)
        self.assertEqual(self.reuse(), 1)


    def test_mismatched_inputs_and_incomplete_never_reuse(self):
        self.assertEqual(self.submit(), 0)
        import json
        path = self.run_dir / 'receipt.json'
        original = json.loads(path.read_text())
        for field, value in [('tree', 'different'), ('complete', False), ('verdict', 'RED'), ('mode', 'build'),
                             ('gate_sha', 'different'), ('runner_sha', 'different'),
                             ('oracle_sha', 'different'), ('schema', 0)]:
            with self.subTest(field=field):
                path.write_text(json.dumps(dict(original, **{field: value})))
                self.assertEqual(self.reuse(), 1)
        path.write_text(json.dumps(original))
        self.assertEqual(self.reuse('--require', 'missing'), 1)
        (self.wt / 'f.rs').write_text('dirty')
        self.assertEqual(self.reuse(), 1)


class ResourceEntry(unittest.TestCase):
    def test_outer_preserves_interpreter_flags_and_propagates_status(self):
        import gate_limits
        with mock.patch.object(gate.sys, 'argv', ['gate', 'full']), \
                mock.patch.object(gate.sys, 'orig_argv', ['python', '-B', 'gate', 'full']), \
                mock.patch.object(gate_limits, 'run', return_value=73) as run, \
                mock.patch.object(gate, 'main') as main:
            self.assertEqual(gate.entry_main(), 73)
            run.assert_called_once_with([gate.sys.executable, '-B', 'gate', 'full'])
            main.assert_not_called()

    def test_inner_does_not_relaunch(self):
        import gate_limits
        with mock.patch.object(gate.sys, 'argv', ['gate', '--inner', 'full']), \
                mock.patch.object(gate_limits, 'run') as run, \
                mock.patch.object(gate, 'main', return_value=23):
            self.assertEqual(gate.entry_main(), 23)
            run.assert_not_called()

    def test_unavailable_normalization_fails_closed(self):
        import gate_limits
        with mock.patch.object(gate.sys, 'argv', ['gate', 'full']), \
                mock.patch.object(gate_limits, 'run', side_effect=FileNotFoundError('manager')), \
                mock.patch.object(gate, 'main') as main, contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(gate.entry_main(), 1)
            main.assert_not_called()
        self.assertIn('resource normalization failed', output.getvalue())


if __name__ == '__main__':
    unittest.main()
