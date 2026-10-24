"""Offline bench-mode assertion tests; no live configuration or processes."""
from copy import deepcopy
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import romacs_accept_host as host


def status():
    actual = {'host': 'fixture', 'online_cpus': '0-3', 'offline_cpus': [], 'stopped_services': ['cups.service'],
              'governors': {'policy0': 'performance'}, 'turbo': {'no_turbo': '0'},
              'frequency_limits': {'max': '2500000'}, 'ac_online': {'AC': '1'},
              'thp': {'enabled': '[always] madvise never', 'defrag': 'always [madvise] never'},
              'services': {'cups.service': 'inactive', 'sshd.service': 'active', 'tailscaled.service': 'active'},
              'cpusets': {name: {'AllowedCPUs': value, 'EffectiveCPUs': value}
                         for name, value in [('system.slice', '0-1'), ('user.slice', '0-1'),
                                             ('init.scope', '0-1'), ('romacs-bench.slice', '2-3')]}}
    return {'fingerprint': {'sha256': host.digest(actual), 'actual': actual},
            'journal': {'phase': 'active', 'profile': 'isolated', 'turbo': 'unchanged',
                        'admission_fingerprint': {'sha256': host.digest(actual), 'actual': deepcopy(actual)}}}


def interval():
    selected = status()
    before = {'protocol': host.PROTOCOL, **selected, 'valid': True, 'contaminated': False, 'errors': [],
              'boot_id': 'fixture', 'monotonic_ns': 1, 'temperatures_c': [55],
              'cpu_ticks': {name: [100, 0, 0, 900, 0, 0, 0, 0] for name in ('cpu0', 'cpu1')},
              'i8042_irqs': {'1': 4, '12': 4}}
    after = deepcopy(before)
    after['monotonic_ns'] = 2
    after['cpu_ticks'] = {name: [110, 0, 0, 990, 0, 0, 0, 0] for name in ('cpu0', 'cpu1')}
    return before, after


class HostTests(unittest.TestCase):
    def test_status_json_command_is_read_only_and_pinned(self):
        selected = status()
        reply = subprocess.CompletedProcess([], 0, json.dumps(selected), '')
        calls = []
        def runner(command, **kwargs):
            calls.append(command)
            return reply
        result = host.verify_host(selected['fingerprint']['sha256'], runner=runner)
        self.assertTrue(result['valid'], result)
        self.assertEqual(calls[0][1:], ['status', '--json'])
        self.assertEqual(calls[0][0], str(Path.home() / 'romacs-bench/bin/bench-mode'))

    def test_missing_owner_or_schema_fails_closed(self):
        def unavailable(*args, **kwargs):
            raise FileNotFoundError('bench-mode')
        self.assertFalse(host.verify_host(runner=unavailable)['valid'])
        selected = status()
        del selected['fingerprint']['actual']['online_cpus']
        with self.assertRaisesRegex(ValueError, 'required configuration'):
            host.assert_status(selected)

    def test_fingerprint_checksum_and_admission_pin(self):
        selected = status()
        selected['fingerprint']['sha256'] = 'bad'
        with self.assertRaisesRegex(ValueError, 'checksum'):
            host.assert_status(selected)
        with self.assertRaisesRegex(ValueError, 'admission'):
            host.assert_status(status(), 'different')

    def test_each_required_control_and_no_old_cpu3_policy(self):
        self.assertEqual(host.cpus(host.assert_status(status())['actual']['online_cpus']), {0, 1, 2, 3})
        mutations = [lambda a: a.update(online_cpus='0-1,3'),
                     lambda a: a['cpusets']['romacs-bench.slice'].update(EffectiveCPUs='3'),
                     lambda a: a['governors'].update(policy0='schedutil'),
                     lambda a: a['turbo'].update(no_turbo='1'),
                     lambda a: a['services'].update({'cups.service': 'active'}),
                     lambda a: a['ac_online'].update(AC='0')]
        for mutate in mutations:
            selected = status()
            mutate(selected['fingerprint']['actual'])
            selected['fingerprint']['sha256'] = host.digest(selected['fingerprint']['actual'])
            with self.assertRaises(ValueError):
                host.assert_status(selected)

    def test_every_fingerprint_change_refuses_interval(self):
        for key in ('thp', 'frequency_limits', 'services', 'online_cpus', 'cpusets'):
            before, after = interval()
            after['fingerprint']['actual'][key] = {'changed': 'changed'}
            after['fingerprint']['sha256'] = host.digest(after['fingerprint']['actual'])
            self.assertTrue(host.assess(before, after)['contaminated'], key)

    def test_clean_activity_heat_and_cpu_intervals(self):
        before, after = interval()
        result = host.assess(before, after)
        self.assertTrue(result['valid'], result)
        self.assertEqual(result['cpu_busy_other'], 10)
        after['i8042_irqs']['1'] += 3
        self.assertTrue(host.assess(before, after)['input_active'])
        after['temperatures_c'] = [95]
        self.assertFalse(host.assess(before, after)['valid'])
        before, after = interval()
        after['cpu_ticks']['cpu0'] = [200, 0, 0, 900, 0, 0, 0, 0]
        self.assertFalse(host.assess(before, after)['valid'])

    def test_missing_reset_evidence_refused(self):
        before, after = interval()
        for key in ('fingerprint', 'journal', 'cpu_ticks', 'i8042_irqs', 'temperatures_c'):
            invalid = deepcopy(after)
            del invalid[key]
            self.assertTrue(host.assess(before, invalid)['contaminated'])
        after['boot_id'] = 'reboot'
        self.assertFalse(host.assess(before, after)['valid'])

    def test_profile_and_turbo_requests(self):
        selected = status()
        host.assert_status(selected)  # isolated leaves turbo enabled
        with self.assertRaisesRegex(ValueError, 'requested'):
            host.assert_status(selected, expected_profile='quiet')
        selected['journal']['profile'] = 'quiet'
        reply = subprocess.CompletedProcess([], 0, json.dumps(selected), '')
        self.assertTrue(host.verify_host(runner=lambda *a, **k: reply, expected_profile='quiet')['valid'])
        before, after = interval()
        for sample in (before, after):
            sample['journal']['profile'] = 'quiet'
            sample['requirement'] = {'expected_profile': 'quiet', 'expected_turbo': 'unchanged'}
        self.assertTrue(host.assess(before, after)['valid'])
        selected = status()
        selected['journal']['turbo'] = 'off'
        with self.assertRaises(ValueError):
            host.assert_status(selected, expected_turbo='off')
        selected['fingerprint']['actual']['turbo']['no_turbo'] = '1'
        selected['fingerprint']['sha256'] = host.digest(selected['fingerprint']['actual'])
        selected['journal']['admission_fingerprint'] = deepcopy(selected['fingerprint'])
        host.assert_status(selected, expected_turbo='off')

    def test_unowned_active_service_and_stopped_restart(self):
        selected = status()
        selected['fingerprint']['actual']['services']['avahi.service'] = 'active'
        selected['fingerprint']['sha256'] = host.digest(selected['fingerprint']['actual'])
        selected['journal']['admission_fingerprint'] = deepcopy(selected['fingerprint'])
        host.assert_status(selected)
        selected['fingerprint']['actual']['services']['cups.service'] = 'active'
        selected['fingerprint']['actual']['stopped_services'] = []
        selected['fingerprint']['sha256'] = host.digest(selected['fingerprint']['actual'])
        with self.assertRaisesRegex(ValueError, 'prepared fingerprint'):
            host.assert_status(selected)

    def test_complete_owner_fingerprint_includes_noop_controls(self):
        from bench_mode_test import MODE, FakeSystem
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root/'state.json'
            fake = FakeSystem(root/'sysroot', state)
            fake.seed('/sys/class/power_supply/AC/online', '1')
            # Already-correct controls are intentionally absent from actions.
            for cpu in range(4):
                fake.seed(f'/sys/devices/system/cpu/cpufreq/policy{cpu}/scaling_governor', 'performance')
            for name in MODE.SLICES:
                fake.cpus[name] = '2,3' if name == 'romacs-bench.slice' else '0,1'
            mode = MODE.BenchMode(state, fake, root/'sysroot', 'benchmark.example.org')
            journal = mode.on('isolated', 'unchanged')
            self.assertFalse(any(a['kind'] in ('file', 'cpuset') for a in journal['actions']))
            original = {'journal': journal, 'fingerprint': mode.fingerprint()}
            host.assert_status(original)
            for key, value in [('governors', {'bad': 'powersave'}), ('cpusets', {'bad': {}}),
                               ('online_cpus', [0, 1, 3]), ('offline_cpus', [2])]:
                changed = deepcopy(original)
                changed['fingerprint']['actual'][key] = value
                changed['fingerprint']['sha256'] = host.digest(changed['fingerprint']['actual'])
                with self.assertRaises(ValueError):
                    host.assert_status(changed)
            mode.off()

    def test_snapshot_only_reads_runtime_observations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixtures = {'proc/sys/kernel/random/boot_id': 'fixture',
                        'proc/stat': 'cpu0 100 0 0 900 0 0 0 0\ncpu1 100 0 0 900 0 0 0 0',
                        'proc/interrupts': '1: 1 0 IO i8042\n12: 1 0 IO i8042',
                        'sys/class/hwmon/hwmon0/name': 'coretemp',
                        'sys/class/hwmon/hwmon0/temp1_input': '55000'}
            for name, value in fixtures.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(value)
            evidence = {'protocol': host.PROTOCOL, **status(), 'valid': True, 'contaminated': False, 'errors': []}
            with patch.object(host, 'verify_host', return_value=evidence):
                self.assertTrue(host.host_snapshot(root)['valid'])
            self.assertEqual({name: (root/name).read_text() for name in fixtures}, fixtures)


if __name__ == '__main__':
    unittest.main()
