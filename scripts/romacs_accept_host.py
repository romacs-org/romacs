"""Assert bench-mode's host fingerprint; never prepare CPUs or services here.

bench-mode is the sole configuration owner. Acceptance consumes its JSON status,
checks the selected admission requirement and pins every interval/chunk to one
canonical fingerprint. Thermal and CPU/IRQ observations are read-only samples.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

PROTOCOL = 'bench-mode-fingerprint-v1'


def mode_tool():
    return Path.home() / 'romacs-bench/bin/bench-mode'


def digest(actual):
    return hashlib.sha256(json.dumps(actual, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def cpus(value):
    if isinstance(value, list):
        return set(value)
    if not isinstance(value, str):
        raise ValueError('invalid online/cpuset CPU encoding')
    result = set()
    for part in value.replace(',', ' ').split():
        ends = part.split('-')
        result.update(range(int(ends[0]), int(ends[-1]) + 1))
    return result


def assert_status(status, expected_sha=None, expected_profile='isolated', expected_turbo='unchanged'):
    fingerprint, journal = status['fingerprint'], status['journal']
    actual = fingerprint['actual']
    required = ('cpusets', 'online_cpus', 'governors', 'turbo', 'thp', 'services',
                'frequency_limits', 'ac_online')
    if any(not actual.get(key) for key in required):
        raise ValueError('bench-mode fingerprint lacks required configuration fields')
    if any(not isinstance(actual[key], dict) for key in required if key != 'online_cpus'):
        raise ValueError('invalid bench-mode fingerprint field type')
    if fingerprint['sha256'] != digest(actual):
        raise ValueError('bench-mode fingerprint checksum mismatch')
    if expected_sha and fingerprint['sha256'] != expected_sha:
        raise ValueError('bench-mode fingerprint changed from admission requirement')
    if not journal or journal['phase'] != 'active' or journal['profile'] != expected_profile:
        raise ValueError('active requested bench-mode profile required')
    if 'offline_cpus' not in actual or not isinstance(actual['offline_cpus'], list):
        raise ValueError('bench-mode fingerprint lacks required configuration: offline_cpus')
    online, offline = cpus(actual['online_cpus']), cpus(actual['offline_cpus'])
    if online & offline:
        raise ValueError('overlapping online/offline CPUs')
    stopped = actual['stopped_services']
    if not isinstance(stopped, list):
        raise ValueError('invalid stopped_services')
    for name in stopped:
        if actual['services'][name] != 'inactive':
            raise ValueError('bench-mode stopped service is running')
    prepared = journal['admission_fingerprint']
    if prepared['sha256'] != digest(prepared['actual']):
        raise ValueError('invalid owner admission fingerprint checksum')
    if fingerprint != prepared:
        raise ValueError('host differs from bench-mode prepared fingerprint')
    if journal['turbo'] != expected_turbo:
        raise ValueError('requested bench-mode turbo policy differs')
    if journal['turbo'] == 'off':
        if any(str(value) != ('1' if name.endswith('no_turbo') else '0')
               for name, value in actual['turbo'].items()):
            raise ValueError('turbo differs from bench-mode off policy')
    elif journal['turbo'] != 'unchanged':
        raise ValueError('unknown bench-mode turbo policy')
    if not any(str(value) == '1' for value in actual['ac_online'].values()):
        raise ValueError('AC power required')
    return fingerprint


def verify_host(expected_sha=None, runner=subprocess.run, *, expected_profile=None, expected_turbo=None):
    requirement = {'expected_profile': expected_profile or os.environ.get('ROMACS_ACCEPT_PROFILE', 'isolated'),
                   'expected_turbo': expected_turbo or os.environ.get('ROMACS_ACCEPT_TURBO', 'unchanged')}
    result = {'protocol': PROTOCOL, 'errors': [], 'requirement': requirement}
    try:
        reply = runner([str(mode_tool()), 'status', '--json'], capture_output=True,
                       text=True, check=True, timeout=30)
        status = json.loads(reply.stdout)
        result['fingerprint'] = assert_status(status, expected_sha or os.environ.get('ROMACS_ACCEPT_ADMISSION_SHA'), **requirement)
        result['journal'] = status['journal']
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        result['errors'].append(str(error))
    result.update(valid=not result['errors'], contaminated=bool(result['errors']))
    return result


def temperatures(root):
    values = []
    for device in (root / 'sys/class/hwmon').glob('hwmon*'):
        if (device / 'name').read_text().strip() in ('coretemp', 'k10temp', 'zenpower'):
            values += [int(path.read_text().strip()) / 1000 for path in device.glob('temp*_input')]
    if not values or any(not 0 < value <= 150 for value in values):
        raise ValueError('missing/invalid CPU temperature evidence')
    return values


def host_snapshot(root=Path('/'), **requirement):
    evidence = verify_host(**requirement)
    try:
        evidence['boot_id'] = (root / 'proc/sys/kernel/random/boot_id').read_text().strip()
        evidence['monotonic_ns'] = time.monotonic_ns()
        evidence['temperatures_c'] = temperatures(root)
        ticks = {}
        for line in (root / 'proc/stat').read_text().splitlines():
            fields = line.split()
            if fields and fields[0] in ('cpu0', 'cpu1'):
                ticks[fields[0]] = [int(value) for value in fields[1:9]]
        if set(ticks) != {'cpu0', 'cpu1'}:
            raise ValueError('missing desktop CPU counters')
        evidence['cpu_ticks'] = ticks
        irq = {}
        for line in (root / 'proc/interrupts').read_text().splitlines():
            if 'i8042' in line and ':' in line:
                number, fields = line.split(':', 1)
                if number.strip() in ('1', '12'):
                    numbers = []
                    for value in fields.split():
                        if not value.isdecimal():
                            break
                        numbers.append(int(value))
                    if not numbers:
                        raise ValueError('missing i8042 counters')
                    irq[number.strip()] = sum(numbers)
        if set(irq) != {'1', '12'}:
            raise ValueError('missing keyboard/trackpoint IRQ evidence')
        evidence['i8042_irqs'] = irq
    except (OSError, ValueError) as error:
        evidence['errors'].append(str(error))
    evidence.update(valid=not evidence['errors'], contaminated=bool(evidence['errors']))
    return evidence


def assess(before, after):
    result = {'protocol': PROTOCOL, 'before': before, 'after': after, 'errors': []}
    try:
        for sample in (before, after):
            if sample['protocol'] != PROTOCOL or not sample['valid'] or sample['contaminated'] or sample['errors']:
                raise ValueError('invalid bench-mode snapshot')
            assert_status({'fingerprint': sample['fingerprint'], 'journal': sample['journal']},
                          **sample.get('requirement', {}))
        if before.get('requirement') != after.get('requirement'):
            raise ValueError('requested bench-mode policy changed mid-sample')
        if before['fingerprint'] != after['fingerprint']:
            raise ValueError('bench-mode fingerprint changed mid-sample')
        if not before['boot_id'] or before['boot_id'] != after['boot_id'] or after['monotonic_ns'] <= before['monotonic_ns']:
            raise ValueError('reset/missing sample clock or boot identity')
        busy = []
        for cpu in ('cpu0', 'cpu1'):
            start, end = before['cpu_ticks'][cpu], after['cpu_ticks'][cpu]
            if len(start) != 8 or len(end) != 8:
                raise ValueError('invalid CPU counters')
            delta = [b-a for a, b in zip(start, end)]
            if min(delta) < 0 or sum(delta) <= 0:
                raise ValueError('reset/missing CPU interval')
            busy.append(100 * (sum(delta)-delta[3]-delta[4]) / sum(delta))
        result['cpu_busy_other'] = max(busy)
        deltas = [after['i8042_irqs'][key]-before['i8042_irqs'][key] for key in ('1', '12')]
        if min(deltas) < 0:
            raise ValueError('IRQ counters reset')
        result['kbd_irq_delta'] = sum(deltas)
        result['input_active'] = sum(deltas) > 0
        result['temp_c'] = max(before['temperatures_c'] + after['temperatures_c'])
        result['fingerprint'] = before['fingerprint']
        if result['cpu_busy_other'] > 60 or result['temp_c'] > 85:
            raise ValueError('thermal/desktop CPU contamination')
    except (KeyError, ValueError, TypeError, ZeroDivisionError) as error:
        result['errors'].append(str(error))
    result.update(valid=not result['errors'], contaminated=bool(result['errors']))
    return result
