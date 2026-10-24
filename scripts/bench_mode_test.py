#!/usr/bin/env python3
"""Host-effect-free tests for the bench-mode transaction and command boundaries."""
import fcntl
import importlib.machinery
import importlib.util
import json
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from typing import Callable
from unittest.mock import patch


LOADER = importlib.machinery.SourceFileLoader('bench_mode', str(Path(__file__).with_name('bench-mode')))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
assert SPEC is not None
MODE = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(MODE)


class FakeSystem:
    def __init__(self, root, state):
        self.root = root
        self.state = state
        self.calls = []
        self.fail: Callable[[list[str]], bool] | None = None
        self.service = {unit: 'inactive' for unit in MODE.NOISE_UNITS}
        self.service.update({'sddm.service': 'active', 'fstrim.timer': 'active',
                             'sshd.service': 'active', 'tailscaled.service': 'active'})
        self.cpus = {unit: '' for unit in MODE.SLICES}
        self.cpus['system.slice'] = '0-3'
        self.seed('/sys/devices/system/cpu/online', '0-3')
        self.seed('/sys/devices/system/cpu/present', '0-3')
        self.seed(MODE.SIBLING_ONLINE, '1')
        for cpu in (2, 3):
            self.seed(f'/sys/devices/system/cpu/cpu{cpu}/topology/core_id', '1')
            self.seed(f'/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list', '2-3')
        for cpu in range(4):
            policy = f'/sys/devices/system/cpu/cpufreq/policy{cpu}/'
            self.seed(policy + 'affected_cpus', str(cpu))
            self.seed(policy + 'scaling_governor', 'powersave')
            self.seed(policy + 'scaling_available_governors', 'powersave performance')
            self.seed(policy + 'scaling_min_freq', '800000')
            self.seed(policy + 'scaling_max_freq', '3200000')
        self.seed(MODE.TURBO_PATHS[0], '0')
        self.seed('/sys/kernel/mm/transparent_hugepage/enabled', 'always [madvise] never')

    def seed(self, path, value):
        target = self.root / path.lstrip('/')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value + '\n')

    def __call__(self, command, *, input=None):
        self.calls.append(command)
        if command[:2] == ['systemctl', 'show']:
            unit = command[2]
            property = command[3].split('=', 1)[1]
            if property == 'ActiveState':
                return self.service.get(unit, 'inactive')
            if property == 'StartLimitBurst':
                return '5'
            if property == 'StartLimitIntervalUSec':
                return '10s'
            return self.cpus[unit] if property == 'AllowedCPUs' else self.cpus[unit] or '0-3'
        if command[2] == 'journalctl':
            return ''
        journal = json.loads(self.state.read_text())
        if journal['phase'] == 'applying':
            assert any(a.get('pending') for a in journal['actions'])
        if self.fail and self.fail(command):
            raise RuntimeError('injected command failure')
        assert command[:2] == ['sudo', '-n']
        if command[2] == 'tee':
            assert input is not None
            action = next(a for a in journal['actions'] if a['target'] == command[3])
            assert action.get('pending')
            self.seed(command[3], input.strip())
            if command[3] == MODE.SIBLING_ONLINE:
                self.seed('/sys/devices/system/cpu/online', '0-3' if input.strip() == '1' else '0-1,3')
                self.seed('/sys/devices/system/cpu/cpufreq/policy2/affected_cpus',
                          '2' if input.strip() == '1' else '')
        elif command[2:4] == ['systemctl', 'set-property']:
            self.cpus[command[5]] = command[6].split('=', 1)[1]
        elif command[2] == 'systemd-run':
            scope = next(c.split('=', 1)[1] for c in command if c.startswith('--unit='))
            assert journal['scope'] == scope
            assert journal['phase'] == 'active' and journal['profile'] in MODE.PROFILES[1:]
            self.service[scope] = 'active'
            return 0
        elif command[2] == 'systemctl':
            unit = command[-1]
            assert unit not in ('sshd.service', 'tailscaled.service')
            if command[3] == 'start' and self.service.get(unit) == 'failed':
                raise RuntimeError('start rate limit requires reset-failed')
            if command[3] != 'kill':
                self.service[unit] = 'active' if command[3] == 'start' else 'inactive'
        else:
            raise AssertionError(command)
        return ''


class ModeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.state = root / 'state.json'
        self.fake = FakeSystem(root / 'sysroot', self.state)
        self.mode = MODE.BenchMode(self.state, self.fake, root / 'sysroot', 'benchmark.example.org')
        self.original = self.mode.fingerprint()

    def test_restarted_service_at_capture_is_not_admitted(self):
        original = self.mode.fingerprint
        calls = 0
        def capture():
            nonlocal calls
            calls += 1
            if calls == 2:
                self.fake.service['fstrim.timer'] = 'active'
            return original()
        with patch.object(self.mode, 'fingerprint', side_effect=capture):
            with self.assertRaisesRegex(RuntimeError, 'complete owner plan'):
                self.mode.on('isolated', 'unchanged')
        self.assertEqual(self.mode.load()['phase'], 'applying')
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_noop_governor_drift_during_preparation_is_not_admitted(self):
        path = '/sys/devices/system/cpu/cpufreq/policy0/scaling_governor'
        self.fake.seed(path, 'performance')
        original = self.mode.fingerprint
        calls = 0
        def capture():
            nonlocal calls
            calls += 1
            if calls == 2:
                self.fake.seed(path, 'powersave')
            return original()
        with patch.object(self.mode, 'fingerprint', side_effect=capture):
            with self.assertRaisesRegex(RuntimeError, 'complete owner plan'):
                self.mode.on('isolated', 'unchanged')
        self.assertEqual(self.mode.load()['phase'], 'applying')
        self.assertFalse(any(a['target'] == path for a in self.mode.load()['actions']))
        self.mode.off()
        self.assertFalse(self.state.exists())

    def test_failed_unit_is_reset_before_restore_start(self):
        self.mode.on('isolated', 'unchanged')
        self.fake.service['fstrim.timer'] = 'failed'
        self.fake.calls.clear()
        self.mode.off()
        reset = ['sudo', '-n', 'systemctl', 'reset-failed', 'fstrim.timer']
        start = ['sudo', '-n', 'systemctl', 'start', 'fstrim.timer']
        self.assertLess(self.fake.calls.index(reset), self.fake.calls.index(start))
        self.assertEqual(self.mode.fingerprint(), self.original)
        self.assertFalse(self.state.exists())

    def test_unloaded_unit_reset_failure_does_not_prevent_restore(self):
        self.fake.service['udisks2.service'] = 'active'
        before = self.mode.fingerprint()
        self.mode.on('isolated', 'unchanged')
        original_runner = self.fake
        def runner(command, **kwargs):
            if 'reset-failed' in command and command[-1] == 'udisks2.service':
                self.fake.calls.append(command)
                raise RuntimeError('Failed to reset failed state of unit udisks2.service: Unit udisks2.service not loaded.')
            return original_runner(command, **kwargs)
        self.mode.runner = runner
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), before)
        self.assertFalse(self.state.exists())

    def test_unloaded_error_does_not_hide_a_failed_unit(self):
        self.mode.on('isolated', 'unchanged')
        self.fake.service['fstrim.timer'] = 'failed'
        original_runner = self.fake
        def runner(command, **kwargs):
            if 'reset-failed' in command and command[-1] == 'fstrim.timer':
                raise RuntimeError('Unit fstrim.timer not loaded.')
            return original_runner(command, **kwargs)
        self.mode.runner = runner
        with self.assertRaisesRegex(RuntimeError, 'restore incomplete'):
            self.mode.off()
        self.mode.runner = self.fake
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_reset_failure_retains_journal_and_other_units_restore(self):
        self.mode.on('isolated', 'unchanged')
        self.fake.service['fstrim.timer'] = 'failed'
        self.fake.fail = lambda c: 'reset-failed' in c and c[-1] == 'fstrim.timer'
        with self.assertRaisesRegex(RuntimeError, 'restore incomplete'):
            self.mode.off()
        self.assertEqual(self.fake.service['fstrim.timer'], 'failed')
        self.assertEqual(self.mode.load()['phase'], 'restoring')
        self.assertEqual(self.mode.read('/sys/devices/system/cpu/cpufreq/policy0/scaling_governor'), 'powersave')
        self.fake.fail = None
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_legacy_display_manager_journal_still_recovers_failed_unit(self):
        journal = {'version': 1, 'host': 'benchmark.example.org', 'profile': 'isolated',
                   'turbo': 'unchanged', 'phase': 'restoring', 'log': [],
                   'actions': [{'kind': 'service', 'target': 'sddm.service',
                                'before': 'active', 'after': 'inactive', 'pending': True}]}
        self.mode.save(journal)
        self.fake.service['sddm.service'] = 'failed'
        self.mode.off()
        self.assertEqual(self.fake.service['sddm.service'], 'active')
        self.assertFalse(self.state.exists())

    def test_explicit_session_stops_desktop_once_across_chunks(self):
        for unit in MODE.DESKTOP_UNITS:
            self.fake.service[unit] = 'active'
        before = self.mode.fingerprint()
        with patch.dict(os.environ, ROMACS_BENCH_SESSION='test-session'):
            self.mode.session_begin('test-session', 'isolated', 'unchanged')
            for _ in range(15):
                self.mode.on('isolated', 'unchanged')
                self.assertEqual(self.fake.service['sddm.service'], 'inactive')
                self.assertTrue(self.mode.off()['retained'])
            self.mode.session_end('test-session')
        for unit in MODE.DESKTOP_UNITS:
            self.assertEqual(self.fake.calls.count(['sudo', '-n', 'systemctl', 'stop', unit]), 1)
            self.assertEqual(self.fake.calls.count(['sudo', '-n', 'systemctl', 'start', unit]), 1)
        self.assertEqual(self.mode.fingerprint(), before)

    def test_foreign_release_cannot_restore_other_participants(self):
        self.mode.session_begin('owner', 'isolated', 'unchanged')
        self.assertTrue(self.mode.session_end('other')['not_member'])
        for operation in (self.mode.off, lambda: self.mode.run(['true'])):
            with self.assertRaisesRegex(RuntimeError, 'session'):
                operation()
        self.mode.session_end('owner')
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_session_crash_during_prepare_recovers_by_token(self):
        self.fake.fail = lambda c: c[-1] == 'sddm.service' and 'stop' in c
        with self.assertRaises(RuntimeError):
            self.mode.session_begin('owner', 'isolated', 'unchanged')
        self.assertIn('owner', self.mode.participants(self.mode.load()))
        self.fake.fail = None
        self.mode.session_end('owner')
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_refcounted_session_joins_and_last_owner_restores(self):
        self.mode.session_begin('first', 'isolated', 'unchanged')
        self.mode.session_begin('second', 'isolated', 'unchanged')
        self.assertEqual(set(self.mode.participants(self.mode.load())), {'first', 'second'})
        with patch.dict(os.environ, ROMACS_BENCH_SESSION='second'):
            self.mode.run(['true'])
            self.mode.off()
        self.assertTrue(self.mode.session_end('first')['retained'])
        self.assertEqual(self.fake.service['sddm.service'], 'inactive')
        self.mode.session_end('second')
        self.assertEqual(self.mode.fingerprint(), self.original)
        self.assertEqual(self.fake.calls.count(['sudo', '-n', 'systemctl', 'stop', 'sddm.service']), 1)
        self.assertEqual(self.fake.calls.count(['sudo', '-n', 'systemctl', 'start', 'sddm.service']), 1)

    def test_different_profile_replaces_preparation_and_old_end_is_noop(self):
        self.mode.session_begin('first', 'isolated', 'unchanged')
        self.mode.session_begin('second', 'quiet', 'off')
        self.assertEqual(self.mode.load()['profile'], 'quiet')
        self.assertEqual(self.mode.load()['turbo'], 'off')
        self.assertEqual(set(self.mode.participants(self.mode.load())), {'second'})
        self.assertTrue(self.mode.session_end('first')['not_member'])
        self.mode.session_end('second')
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_expired_lease_reclaimed_by_next_lane(self):
        with patch.object(MODE.time, 'time', return_value=1000):
            self.mode.session_begin('crashed', 'isolated', 'unchanged')
        with patch.object(MODE.time, 'time', return_value=1000 + MODE.SESSION_LEASE_SECONDS):
            self.mode.session_begin('next', 'isolated', 'unchanged')
        self.assertEqual(set(self.mode.participants(self.mode.load())), {'next'})
        self.assertEqual(self.fake.calls.count(['sudo', '-n', 'systemctl', 'start', 'sddm.service']), 1)
        self.mode.session_end('next')
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_mixed_expiry_preserves_live_peers_and_single_preparation(self):
        with patch.object(MODE.time, 'time', return_value=1000):
            self.mode.session_begin('crashed', 'isolated', 'unchanged')
        with patch.object(MODE.time, 'time', return_value=2800):
            self.mode.session_begin('live-B', 'isolated', 'unchanged')
        with patch.object(MODE.time, 'time', return_value=3000):
            before = self.mode.session_begin('live-C', 'isolated', 'unchanged')
        with patch.object(MODE.time, 'time', return_value=4600):
            joined = self.mode.session_begin('live-B', 'isolated', 'unchanged')
            self.assertEqual(set(self.mode.participants(joined)), {'live-B', 'live-C'})
            self.assertEqual(joined['actions'], before['actions'])
            self.assertEqual(joined['admission_fingerprint'], before['admission_fingerprint'])
            self.assertEqual(self.mode.participants(joined)['live-B']['expires_at'], 8200)
            self.assertEqual(self.mode.participants(joined)['live-C']['expires_at'], 6600)
            self.assertEqual(self.fake.calls.count(['sudo', '-n', 'systemctl', 'stop', 'sddm.service']), 1)
            self.assertNotIn(['sudo', '-n', 'systemctl', 'start', 'sddm.service'], self.fake.calls)
            self.assertTrue(self.mode.session_end('live-B')['retained'])
            self.mode.session_end('live-C')
        self.assertEqual(self.mode.fingerprint(), self.original)
        self.assertEqual(self.fake.calls.count(['sudo', '-n', 'systemctl', 'start', 'sddm.service']), 1)

    def test_final_live_release_reaps_expired_peer_and_restores(self):
        with patch.object(MODE.time, 'time', return_value=1000):
            self.mode.session_begin('crashed', 'isolated', 'unchanged')
        with patch.object(MODE.time, 'time', return_value=2800):
            self.mode.session_begin('live', 'isolated', 'unchanged')
        with patch.object(MODE.time, 'time', return_value=4600):
            self.mode.session_end('live')
        self.assertIsNone(self.mode.load())
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_explicit_off_cleans_scope_before_retaining_preparation(self):
        self.mode.session_begin('owner', 'isolated', 'unchanged')
        scope = 'romacs-bench-' + 'a' * 32 + '.scope'
        journal = self.mode.load()
        journal['scope'] = scope
        self.mode.save(journal)
        self.fake.service[scope] = 'active'
        with patch.dict(os.environ, ROMACS_BENCH_SESSION='owner'):
            self.fake.fail = lambda c: c[-1] == scope and ('kill' in c or 'stop' in c)
            with self.assertRaisesRegex(RuntimeError, 'not inactive'):
                self.mode.off()
            self.assertEqual(self.mode.load()['scope'], scope)
            self.fake.fail = None
            self.assertTrue(self.mode.off()['retained'])
        self.assertNotIn('scope', self.mode.load())
        self.assertEqual(self.fake.service[scope], 'inactive')
        self.assertEqual(self.fake.service['sddm.service'], 'inactive')
        self.mode.session_end('owner')

    def test_real_priority_queue_interleaves_joined_and_legacy_chunks(self):
        import time
        root = self.state.parent / 'queued'
        root.mkdir()
        driver = root / 'driver.py'
        driver.write_text('''import json, os, pickle, sys
from pathlib import Path
sys.path.insert(0, SCRIPTS)
from bench_mode_test import FakeSystem, MODE
from romacs_accept_queue import run_ticket
root = Path(__file__).parent
operation = sys.argv[1]
if operation == 'ticket':
    step = sys.argv[2]
    rank = 10 if step == 'B' else 50
    raise SystemExit(run_ticket(root, step, rank, 30, [sys.executable, __file__, step]))
MODE.validate_lock(root / '.lock')
snapshot = root / 'fake.pickle'
if snapshot.exists():
    fake = pickle.loads(snapshot.read_bytes())
else:
    fake = FakeSystem(root / 'sysroot', root / 'state.json')
mode = MODE.BenchMode(root / 'state.json', fake, root / 'sysroot', 'benchmark.example.org')
if operation == 'A1':
    (root / 'original.json').write_text(json.dumps(mode.fingerprint()))
token = 'B' if operation == 'B' else 'A'
os.environ['ROMACS_BENCH_SESSION'] = token
mode.session_begin(token, 'isolated', 'unchanged')
assert mode.run(['true']) == 0
mode.off()
with (root / 'order').open('a') as stream:
    stream.write(operation + '\\n')
if operation == 'B':
    assert len(mode.participants(mode.load())) == 2
    assert mode.session_end('B')['retained']
    os.environ.pop('ROMACS_BENCH_SESSION')
    mode.on('isolated', 'unchanged')
    assert mode.run(['true']) == 0
    assert mode.off()['retained']
    assert set(mode.participants(mode.load())) == {'A'}
    with (root / 'order').open('a') as stream:
        stream.write('legacy\\n')
if operation == 'A2':
    mode.session_end('A')
    assert mode.load() is None
    assert mode.fingerprint() == json.loads((root / 'original.json').read_text())
    for action in ('stop', 'start'):
        assert fake.calls.count(['sudo', '-n', 'systemctl', action, 'sddm.service']) == 1
snapshot.write_bytes(pickle.dumps(fake))
'''.replace('SCRIPTS', repr(str(Path(__file__).resolve().parent))))
        result = subprocess.run([sys.executable, str(driver), 'ticket', 'A1'], capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        (root / 'PAUSE').touch()
        children = [subprocess.Popen([sys.executable, str(driver), 'ticket', step],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    for step in ('A2', 'B')]
        try:
            deadline = time.monotonic() + 10
            while len(list((root / 'queue').glob('*.json'))) != 2:
                self.assertLess(time.monotonic(), deadline, 'tickets did not publish')
                time.sleep(0.02)
            (root / 'PAUSE').unlink()
            for child in children:
                out, error = child.communicate(timeout=30)
                self.assertEqual(child.returncode, 0, out + error)
        finally:
            for child in children:
                if child.poll() is None:
                    child.kill()
                    child.communicate()
        self.assertEqual((root / 'order').read_text().splitlines(), ['A1', 'B', 'legacy', 'A2'])

    def test_recent_unit_starts_refuse_before_any_mutation(self):
        original = self.fake
        def runner(command, **kwargs):
            if command[2] == 'journalctl':
                return '\n'.join(json.dumps({'_PID': '1', 'JOB_TYPE': 'start',
                                             'JOB_ID': str(index), 'UNIT': 'sddm.service'})
                                 for index in range(4))
            return original(command, **kwargs)
        self.mode.runner = runner
        with self.assertRaisesRegex(RuntimeError, 'recent starts 4 near StartLimitBurst=5'):
            self.mode.session_begin('owner', 'isolated', 'unchanged')
        self.assertFalse(self.state.exists())
        self.assertEqual(self.mode.fingerprint(), self.original)
        self.assertFalse(any('stop' in c or 'tee' in c or 'reset-failed' in c for c in self.fake.calls))

    def test_simple_service_started_only_jobs_count_and_duplicates_do_not(self):
        original = self.fake
        def runner(command, **kwargs):
            if command[2] == 'journalctl':
                return '\n'.join(json.dumps({'_PID': '1', 'JOB_TYPE': 'start', 'JOB_ID': '123',
                                             'MESSAGE_ID': message, 'UNIT': 'sddm.service'})
                                 for message in ('39f53479d3a045ac8e11786248231fbf',) * 8)
            return original(command, **kwargs)
        self.mode.runner = runner
        self.mode.session_begin('owner', 'isolated', 'unchanged')
        self.mode.session_end('owner')
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_failed_slice_is_reset_before_restore_start(self):
        self.fake.service['romacs-bench.slice'] = 'active'
        action = {'kind': 'slice', 'target': 'romacs-bench.slice',
                  'before': 'active', 'after': 'inactive', 'pending': True}
        self.mode.save({'version': 1, 'host': 'benchmark.example.org', 'profile': 'isolated',
                        'turbo': 'unchanged', 'phase': 'restoring', 'actions': [action], 'log': []})
        self.fake.service['romacs-bench.slice'] = 'failed'
        self.mode.off()
        self.assertEqual(self.fake.service['romacs-bench.slice'], 'active')

    def test_quiet_exact_restore_and_idempotence(self):
        self.mode.on('quiet', 'unchanged')
        mutations = [c for c in self.fake.calls if c[0] == 'sudo']
        self.assertFalse(any('set-property' in c for c in mutations))
        self.assertFalse(any('no_turbo' in ' '.join(c) for c in mutations))
        self.mode.on('quiet', 'unchanged')
        self.assertEqual(mutations, [c for c in self.fake.calls if c[0] == 'sudo'])
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)
        self.assertFalse(self.state.exists())
        self.assertEqual(self.mode.off(), {'phase': 'off'})

    def test_isolated_turbo_exact_restore(self):
        self.mode.on('isolated', 'off')
        self.assertEqual(self.fake.cpus['system.slice'], '0,1')
        self.assertEqual(self.fake.cpus['user.slice'], '0,1')
        self.assertEqual(self.fake.cpus['init.scope'], '0,1')
        self.assertEqual(self.fake.cpus['romacs-bench.slice'], '2,3')
        self.assertEqual(self.mode.read(MODE.TURBO_PATHS[0]), '1')
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_sibling_offline_exact_restore_and_cpu3_run(self):
        self.mode.on('isolated+sibling-offline', 'off')
        self.assertEqual(self.mode.read(MODE.SIBLING_ONLINE), '0')
        self.assertEqual(self.mode.fingerprint()['actual']['online_cpus'], [0, 1, 3])
        self.assertEqual(self.fake.cpus['romacs-bench.slice'], '3')
        self.assertEqual(self.fake.cpus['system.slice'], '0,1')
        with patch.dict(os.environ, {'SUDO_UID': '1000'}):
            self.assertEqual(self.mode.run(['/bin/true']), 0)
        launch = next(c for c in self.fake.calls if 'systemd-run' in c)
        self.assertIn('--property=AllowedCPUs=3', launch)
        self.assertEqual(launch[launch.index('taskset') + 2], '3')
        self.assertIn('if cpus != {3}:', launch[-3])
        self.fake.calls.clear()
        self.mode.off()
        self.assertEqual(self.fake.calls[0], ['sudo', '-n', 'tee', MODE.SIBLING_ONLINE])
        self.assertEqual(self.mode.fingerprint(), self.original)
        self.assertFalse(any('/cpu0/online' in ' '.join(c) for c in self.fake.calls))

    def test_sibling_topology_and_online_preflight_fail_without_mutation(self):
        for path, value in ((MODE.SIBLING_ONLINE, '0'),
                            ('/sys/devices/system/cpu/online', '0-1,3'),
                            ('/sys/devices/system/cpu/cpu3/topology/core_id', '0'),
                            ('/sys/devices/system/cpu/cpu2/topology/thread_siblings_list', '0,2')):
            with self.subTest(path=path):
                before = self.mode.read(path)
                self.fake.seed(path, value)
                with self.assertRaises(RuntimeError):
                    self.mode.on('isolated+sibling-offline', 'unchanged')
                self.assertFalse(self.state.exists())
                self.assertFalse(any(c[0] == 'sudo' for c in self.fake.calls))
                self.fake.seed(path, before)

    def test_hotplug_effect_before_applied_marker_recovers(self):
        original_apply = self.mode.apply

        def interrupted(action, restore=False):
            original_apply(action, restore)
            if action['kind'] == 'cpu-online' and not restore:
                raise RuntimeError('crash after hotplug')

        with patch.object(self.mode, 'apply', side_effect=interrupted):
            with self.assertRaisesRegex(RuntimeError, 'crash after hotplug'):
                self.mode.on('isolated+sibling-offline', 'unchanged')
        action = self.mode.load()['actions'][-1]
        self.assertTrue(action['pending'])
        self.assertNotIn('applied', action)
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_offline_cpu_restore_failure_retains_journal_and_retries(self):
        self.mode.on('isolated+sibling-offline', 'unchanged')
        self.fake.fail = lambda c: MODE.SIBLING_ONLINE in c
        with self.assertRaisesRegex(RuntimeError, 'restore incomplete'):
            self.mode.off()
        self.assertTrue(self.state.exists())
        self.assertEqual(self.mode.read(MODE.SIBLING_ONLINE), '0')
        self.fake.fail = None
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_restored_cpu_is_rechecked_after_other_restore_failure(self):
        self.mode.on('isolated+sibling-offline', 'unchanged')
        self.fake.fail = lambda c: 'start' in c and 'fstrim.timer' in c
        with self.assertRaisesRegex(RuntimeError, 'restore incomplete'):
            self.mode.off()
        self.assertTrue(self.mode.load()['actions'][-1]['restored'])
        self.fake.seed(MODE.SIBLING_ONLINE, '0')
        self.fake.seed('/sys/devices/system/cpu/online', '0-1,3')
        self.fake.fail = None
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_restore_never_completes_with_another_offline_cpu(self):
        self.mode.on('isolated+sibling-offline', 'unchanged')
        original_runner = self.fake

        def runner(command, **kwargs):
            result = original_runner(command, **kwargs)
            if command[-1] == 'fstrim.timer' and 'start' in command:
                self.fake.seed('/sys/devices/system/cpu/online', '0,2-3')
            return result

        self.mode.runner = runner
        with self.assertRaisesRegex(RuntimeError, 'CPUs 0,1,2,3 must be online'):
            self.mode.off()
        self.assertTrue(self.state.exists())
        self.mode.runner = self.fake
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_status_json_retains_fingerprint_shape_and_explicit_actual_fields(self):
        with patch.object(MODE, 'BenchMode', return_value=self.mode), patch('sys.stdout', new_callable=io.StringIO) as output:
            self.assertEqual(MODE.main(['status', '--json']), 0)
        status = json.loads(output.getvalue())
        self.assertEqual(set(status), {'journal', 'fingerprint'})
        self.assertEqual(status['fingerprint'], self.original)
        actual = status['fingerprint']['actual']
        self.assertEqual(actual['online_cpus'], [0, 1, 2, 3])
        for field in ('cpusets', 'governors', 'turbo', 'thp', 'stopped_services', 'graphical_target'):
            self.assertIn(field, actual)
        self.fake.service['graphical.target'] = 'active'
        self.assertNotEqual(self.mode.fingerprint()['sha256'], self.original['sha256'])

    def test_offline_policy_fingerprint_and_status_cli(self):
        self.mode.on('isolated+sibling-offline', 'unchanged')
        original_read = Path.read_text

        def read(path, *args, **kwargs):
            if path.parent.name == 'policy2':
                raise OSError(16, 'Device or resource busy')
            return original_read(path, *args, **kwargs)

        with patch.object(Path, 'read_text', read):
            for command in (['fingerprint'], ['status', '--json']):
                with self.subTest(command=command), patch.object(MODE, 'BenchMode', return_value=self.mode), patch('sys.stdout', new_callable=io.StringIO) as output:
                    self.assertEqual(MODE.main(command), 0)
                result = json.loads(output.getvalue())
                fingerprint = result['fingerprint'] if command[0] == 'status' else result
                self.assertEqual(set(fingerprint), {'sha256', 'actual'})
                self.assertEqual(len(fingerprint['sha256']), 64)
                actual = fingerprint['actual']
                self.assertEqual(actual['online_cpus'], [0, 1, 3])
                self.assertEqual(actual['offline_cpus'], [2])
                self.assertFalse(any('policy2/' in key for field in ('governors', 'frequency_limits') for key in actual[field]))
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_policy_read_errors_other_than_inactive_busy_are_not_hidden(self):
        original_read = Path.read_text

        def read(path, *args, **kwargs):
            if path.parent.name == 'policy2':
                raise PermissionError(13, 'Permission denied')
            return original_read(path, *args, **kwargs)

        with patch.object(Path, 'read_text', read), self.assertRaises(PermissionError):
            self.mode.fingerprint()

    def test_removed_offline_policy_is_not_required(self):
        self.fake.seed('/sys/devices/system/cpu/online', '0-1,3')
        policy = self.mode.path('/sys/devices/system/cpu/cpufreq/policy2')
        for child in policy.iterdir():
            child.unlink()
        policy.rmdir()
        actual = self.mode.fingerprint()['actual']
        self.assertEqual(actual['offline_cpus'], [2])
        self.assertEqual(len(actual['governors']), 3)

    def test_added_noise_services_are_stopped_and_restored_only_if_active(self):
        units = ('pacman-init.timer', 'pacman-keyring-wkd-sync.timer')
        for unit in units:
            self.fake.service[unit] = 'active'
        before = self.mode.fingerprint()
        self.mode.on('quiet', 'unchanged')
        self.assertTrue(all(self.fake.service[unit] == 'inactive' for unit in units))
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), before)

    def test_failure_keeps_journal_and_off_restores_partial_changes(self):
        self.fake.fail = lambda c: c[-1] == 'fstrim.timer'
        with self.assertRaisesRegex(RuntimeError, 'injected'):
            self.mode.on('quiet', 'unchanged')
        self.assertTrue(self.state.exists())
        with self.assertRaisesRegex(RuntimeError, 'existing journal'):
            self.mode.on('quiet', 'unchanged')
        self.fake.fail = None
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_restore_failure_retains_retryable_journal_and_log(self):
        self.mode.on('isolated', 'off')
        self.fake.fail = lambda c: 'start' in c and 'fstrim.timer' in c
        with self.assertRaisesRegex(RuntimeError, 'restore incomplete'):
            self.mode.off()
        journal = json.loads(self.state.read_text())
        self.assertEqual(journal['phase'], 'restoring')
        self.assertTrue(journal['log'])
        self.assertTrue(any(a.get('restored') for a in journal['actions']))
        self.fake.fail = None
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_crash_after_effect_before_applied_marker_is_restorable(self):
        journal = {'version': 1, 'host': 'benchmark.example.org', 'profile': 'quiet', 'turbo': 'unchanged',
                   'phase': 'applying', 'actions': self.mode.plan('quiet', 'unchanged'), 'log': []}
        journal['actions'][0]['pending'] = True
        self.mode.save(journal)
        self.mode.apply(journal['actions'][0])
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_fingerprint_is_actual_stable_and_sensitive(self):
        self.assertEqual(self.original, self.mode.fingerprint())
        self.fake.service['sshd.service'] = 'inactive'
        self.assertNotEqual(self.original['sha256'], self.mode.fingerprint()['sha256'])
        self.assertNotIn('timestamp', json.dumps(self.original))

    def test_transitioning_service_fails_before_mutation(self):
        self.fake.service['fstrim.timer'] = 'deactivating'
        with self.assertRaisesRegex(RuntimeError, 'transitioning'):
            self.mode.on('quiet', 'unchanged')
        self.assertFalse(self.state.exists())
        self.assertFalse(any(c[0] == 'sudo' for c in self.fake.calls))

    def test_profile_change_requires_off(self):
        self.mode.on('quiet', 'unchanged')
        with self.assertRaisesRegex(RuntimeError, 'existing journal'):
            self.mode.on('isolated', 'off')

    def test_no_permanent_or_connectivity_mutations(self):
        self.mode.on('isolated', 'off')
        self.mode.off()
        commands = [c for c in self.fake.calls if c[0] == 'sudo']
        for command in commands:
            self.assertFalse({'mask', 'isolate', 'reboot', 'suspend', 'sshd.service', 'tailscaled.service'} & set(command))
            if 'set-property' in command:
                self.assertIn('--runtime', command)

    def test_run_scope_outside_system_slice_with_user_credentials(self):
        calls = []
        with patch.dict(os.environ, {'SUDO_UID': '1000'}, clear=True), patch.object(MODE.pwd, 'getpwuid') as user:
            user.return_value.pw_name = 'bench-user'
            MODE.run_command(['/bin/true'], 'romacs-bench-' + 'a' * 32 + '.scope', lambda args: calls.append(args))
        command = calls[0]
        self.assertIn('--slice=romacs-bench.slice', command)
        self.assertIn('--property=AllowedCPUs=2,3', command)
        self.assertIn('--unit=romacs-bench-' + 'a' * 32 + '.scope', command)
        self.assertEqual(command[command.index('taskset'):][-8:-4], ['taskset', '-c', '2,3', 'python3'])
        self.assertEqual(command[-3:], [MODE.AFFINITY_WRAPPER, 'romacs-bench-' + 'a' * 32 + '.scope', '/bin/true'])

    def test_run_preserves_benchmark_environment_after_privilege_drop(self):
        calls = []
        with patch.dict(os.environ, {'SUDO_UID': '1000', 'ROMACS_RUNTIME_ROOT': '/runtime',
                                      'CORE_BENCH_XDISP': '/fixture', 'ROMACS_BENCH_LOCK_FD': '9'}, clear=True), patch.object(MODE.pwd, 'getpwuid') as user:
            user.return_value.pw_name = 'bench-user'
            MODE.run_command(['/bin/true'], 'romacs-bench-' + 'a' * 32 + '.scope', lambda args: calls.append(args))
        command = calls[0]
        self.assertIn('ROMACS_RUNTIME_ROOT=/runtime', command)
        self.assertIn('CORE_BENCH_XDISP=/fixture', command)
        self.assertNotIn('ROMACS_BENCH_LOCK_FD=9', command)
        self.assertLess(command.index('bench-user'), command.index('env'))

    def test_run_requires_active_isolated_journal(self):
        for profile in (None, 'quiet', 'isolated'):
            with self.subTest(profile=profile):
                if profile:
                    journal = self.mode.on(profile, 'unchanged')
                    if profile == 'isolated':
                        journal['phase'] = 'applying'
                        self.mode.save(journal)
                with self.assertRaisesRegex(RuntimeError, 'active isolated'):
                    self.mode.run(['/bin/true'])
                self.mode.off()

    def test_run_journals_exact_scope_before_launch_and_stops_afterward(self):
        self.mode.on('isolated', 'off')
        with patch.dict(os.environ, {'SUDO_UID': '1000'}):
            self.assertEqual(self.mode.run(['/bin/true']), 0)
        launch = next(c for c in self.fake.calls if 'systemd-run' in c)
        scope = next(c.split('=', 1)[1] for c in launch if c.startswith('--unit='))
        kills = [c for c in self.fake.calls if 'kill' in c]
        self.assertEqual(kills, [['sudo', '-n', 'systemctl', 'kill', '--signal=SIGKILL', '--kill-whom=all', scope]])
        self.assertEqual(self.fake.service[scope], 'inactive')
        journal = self.mode.load()
        self.assertNotIn('scope', journal)
        self.assertEqual(journal['log'][-2:], [{'operation': 'scope-launch', 'target': scope},
                                             {'operation': 'scope-stopped', 'target': scope}])

    def track_scope(self):
        journal = self.mode.on('isolated', 'off')
        scope = 'romacs-bench-' + 'a' * 32 + '.scope'
        journal['scope'] = scope
        self.mode.save(journal)
        self.fake.service[scope] = 'active'
        self.fake.calls.clear()
        return scope

    def test_off_stops_only_tracked_scope_before_any_restore(self):
        scope = self.track_scope()
        unrelated = 'romacs-bench-' + 'b' * 32 + '.scope'
        self.fake.service[unrelated] = 'active'
        self.mode.off()
        self.assertEqual(self.fake.calls[:3], [
            ['sudo', '-n', 'systemctl', 'kill', '--signal=SIGKILL', '--kill-whom=all', scope],
            ['sudo', '-n', 'systemctl', 'stop', scope],
            ['systemctl', 'show', scope, '--property=ActiveState', '--value']])
        self.assertEqual(self.fake.service[unrelated], 'active')
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_scope_cleanup_failure_prevents_all_mode_restore_and_keeps_journal(self):
        scope = self.track_scope()
        self.fake.fail = lambda c: 'stop' in c and c[-1] == scope
        with self.assertRaisesRegex(RuntimeError, 'scope not inactive'):
            self.mode.off()
        self.assertEqual(len(self.fake.calls), 3)
        self.assertEqual(self.mode.load()['scope'], scope)
        self.assertEqual(self.fake.cpus['romacs-bench.slice'], '2,3')
        self.assertEqual(self.mode.read(MODE.TURBO_PATHS[0]), '1')
        self.fake.fail = None
        self.mode.off()
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_failed_scope_is_reset_exactly_before_restore(self):
        scope = self.track_scope()
        self.fake.service[scope] = 'failed'
        original_runner = self.fake

        def runner(command, **kwargs):
            if command[2:4] == ['systemctl', 'stop'] and command[-1] == scope:
                self.fake.calls.append(command)
                return ''
            return original_runner(command, **kwargs)

        self.mode.runner = runner
        self.mode.off()
        self.assertEqual(self.fake.calls[:5], [
            ['sudo', '-n', 'systemctl', 'kill', '--signal=SIGKILL', '--kill-whom=all', scope],
            ['sudo', '-n', 'systemctl', 'stop', scope],
            ['systemctl', 'show', scope, '--property=ActiveState', '--value'],
            ['sudo', '-n', 'systemctl', 'reset-failed', scope],
            ['systemctl', 'show', scope, '--property=ActiveState', '--value']])
        self.assertEqual(self.mode.fingerprint(), self.original)

    def test_real_sigterm_cleans_scope_and_restores_signal_handler(self):
        self.mode.on('isolated', 'off')
        original_runner = self.fake
        original_handler = MODE.signal.getsignal(MODE.signal.SIGTERM)

        def runner(command, **kwargs):
            result = original_runner(command, **kwargs)
            if 'systemd-run' in command:
                os.kill(os.getpid(), MODE.signal.SIGTERM)
            return result

        self.mode.runner = runner
        with patch.dict(os.environ, {'SUDO_UID': '1000'}):
            with self.assertRaisesRegex(RuntimeError, 'interrupted by signal'):
                self.mode.run(['/bin/true'])
        self.assertNotIn('scope', self.mode.load())
        self.assertEqual(MODE.signal.getsignal(MODE.signal.SIGTERM), original_handler)
        self.assertTrue(all(state == 'inactive' for unit, state in self.fake.service.items() if unit.endswith('.scope')))

    def test_invalid_scope_never_reaches_systemctl_or_restore(self):
        self.track_scope()
        journal = self.mode.load()
        journal['scope'] = 'sshd.service'
        self.mode.save(journal)
        with self.assertRaisesRegex(RuntimeError, 'invalid tracked'):
            self.mode.off()
        self.assertEqual(self.fake.calls, [])

    def test_run_failure_still_stops_launched_scope(self):
        self.mode.on('isolated', 'off')
        original_runner = self.fake

        def runner(command, **kwargs):
            result = original_runner(command, **kwargs)
            if 'systemd-run' in command:
                raise RuntimeError('interrupted launch')
            return result

        self.mode.runner = runner
        with patch.dict(os.environ, {'SUDO_UID': '1000'}):
            with self.assertRaisesRegex(RuntimeError, 'interrupted launch'):
                self.mode.run(['/bin/true'])
        self.assertNotIn('scope', self.mode.load())
        self.assertTrue(all(state == 'inactive' for unit, state in self.fake.service.items() if unit.endswith('.scope')))

    def test_host_guard_cli_has_no_mutation(self):
        with patch.object(MODE.socket, 'gethostname', return_value='other-benchmark.example.org'), patch.object(MODE, 'execute') as runner:
            self.assertEqual(MODE.main(['--host', 'benchmark.example.org', 'on']), 1)
            runner.assert_not_called()

    def test_host_guard_requires_explicit_host_before_mutation(self):
        with patch.object(MODE.socket, 'gethostname', return_value='benchmark.example.org'), patch.object(MODE, 'execute') as runner, patch('sys.stderr', new_callable=io.StringIO) as error:
            self.assertEqual(MODE.main(['on']), 1)
            self.assertIn('--host is required', error.getvalue())
            runner.assert_not_called()


class AffinityAssertionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.receipt = self.root / 'affinity.json'
        self.executed = self.root / 'executed'
        self.scope = 'romacs-bench-' + 'a' * 32 + '.scope'
        self.cgroup = '0::/romacs.slice/romacs-bench.slice/' + self.scope + '\n'

    def invoke(self, cpus, cgroup, wrapper=None):
        code = ('import os, sys; from pathlib import Path; from unittest.mock import patch; '
                + 'sys.argv=' + repr(['-c', self.scope, 'python3', '-c',
                                      'from pathlib import Path; Path(' + repr(str(self.executed)) + ').write_text("executed")']) + '; '
                + 'a=patch.object(os,"sched_getaffinity",return_value=' + repr(cpus) + '); a.start(); '
                + 'b=patch.object(Path,"read_text",return_value=' + repr(cgroup) + '); b.start(); '
                + 'exec(' + repr(MODE.AFFINITY_WRAPPER if wrapper is None else wrapper) + ')')
        return subprocess.run(['python3', '-c', code], capture_output=True, text=True,
                              env=dict(os.environ, ROMACS_BENCH_AFFINITY_RECEIPT=str(self.receipt)))

    def test_production_wrapper_receipts_then_execs(self):
        result = self.invoke({2, 3}, self.cgroup)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.executed.read_text(), 'executed')
        receipt = json.loads(self.receipt.read_text())
        self.assertEqual(receipt['affinity'], [2, 3])
        self.assertEqual(receipt['scope'], self.scope)
        self.assertEqual(receipt['cgroup'], self.cgroup)
        self.assertGreater(receipt['pid'], 0)

    def test_production_wrapper_rejects_wrong_actual_affinity_before_exec(self):
        for cpus in ({0, 1}, {2}, {0, 1, 2, 3}):
            with self.subTest(cpus=cpus):
                result = self.invoke(cpus, self.cgroup)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('affinity assertion failed', result.stderr)
                self.assertFalse(self.receipt.exists())
                self.assertFalse(self.executed.exists())

    def test_production_wrapper_rejects_wrong_group_and_scope_before_exec(self):
        for cgroup in ('0::/system.slice/' + self.scope + '\n',
                       '0::/romacs.slice/romacs-bench.slice/other.scope\n',
                       '0::/system.slice/romacs-bench.slice/' + self.scope + '\n',
                       '0::/romacs.slice/romacs-bench.slice/' + self.scope + '/child\n'):
            with self.subTest(cgroup=cgroup):
                result = self.invoke({2, 3}, cgroup)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('cgroup assertion failed', result.stderr)
                self.assertFalse(self.receipt.exists())
                self.assertFalse(self.executed.exists())

    def test_wrong_cpu_mutant_is_observably_unsafe(self):
        mutant = MODE.AFFINITY_WRAPPER.replace('if cpus != {2, 3}:', 'if False:')
        result = self.invoke({0, 1}, self.cgroup, mutant)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.executed.exists())
        self.assertEqual(json.loads(self.receipt.read_text())['affinity'], [0, 1])

    def test_wrong_group_mutant_is_observably_unsafe(self):
        mutant = MODE.AFFINITY_WRAPPER.replace("if len(paths) != 1 or paths[0].split('/')[-2:] != ['romacs-bench.slice', scope] or 'system.slice' in paths[0].split('/'):", 'if False:')
        result = self.invoke({2, 3}, '0::/system.slice/wrong.scope\n', mutant)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.executed.exists())


class LockTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / '.lock'
        self.lock = self.path.open('w+')
        self.addCleanup(self.lock.close)

    def test_real_inherited_lock_is_valid(self):
        fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.assertEqual(MODE.validate_lock(self.path, self.lock.fileno()), self.lock.fileno())

    def test_same_inode_without_lock_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, 'not held'):
            MODE.validate_lock(self.path, self.lock.fileno())

    def test_fake_different_inode_is_rejected(self):
        other = self.path.with_name('fake')
        with other.open('w+') as stream:
            fcntl.flock(stream, fcntl.LOCK_EX)
            with self.assertRaisesRegex(RuntimeError, 'does not name'):
                MODE.validate_lock(self.path, stream.fileno())

    def test_lock_held_by_different_open_description_is_rejected(self):
        fcntl.flock(self.lock, fcntl.LOCK_EX)
        with self.path.open('r') as wrong:
            with self.assertRaises(RuntimeError):
                MODE.validate_lock(self.path, wrong.fileno())

    def test_inherited_descriptor_survives_real_subprocess(self):
        fcntl.flock(self.lock, fcntl.LOCK_EX)
        code = ('import importlib.machinery; m=importlib.machinery.SourceFileLoader("mode", '
                + repr(str(Path(__file__).with_name('bench-mode'))) + ').load_module(); '
                + 'from pathlib import Path; m.validate_lock(Path(' + repr(str(self.path)) + '))')
        env = dict(os.environ, ROMACS_BENCH_LOCK_FD=str(self.lock.fileno()))
        result = subprocess.run(['python3', '-c', code], env=env, pass_fds=(self.lock.fileno(),), capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__':
    unittest.main()
