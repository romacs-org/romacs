#!/usr/bin/env python3
"""Terminal typing latency, base against candidate: a host key's ingress to
its byte's arrival at the terminal's pty, and frames per key.

Each session is a fresh private headless labwc and a fresh `romacs-vm-gui
-Q [--loadup] --trace-latency` that opens `romacs-term` with M-x.  In the
terminal a raw-mode reader stamps each read with CLOCK_MONOTONIC; the
binary's `key-ingress` records stamp each pressed key's enqueue on the same
clock.  A session runs two phases, each N lowercase letters typed with
wtype D ms apart and ended by C-]:

  typing  the reader only reads;
  output  the reader also writes a line to the terminal every 5 ms.

The i-th key's latency is the i-th byte's arrival less the i-th ingress
after the phase starts; a phase whose bytes are not exactly the letters
typed is refused.  Frames per key are the presentations submitted during
the phase over N.

Sessions alternate base and candidate for PAIRS pairs.  The report gives,
per phase and binary, the median over sessions of each session's p50 and
p95, each pair's deltas, and the base spread (the base sessions' p50 range
over their median).  A phase PASSES when the candidate's p50 and p95 are
within BAR of base's, its frames per key exceed base's by under 0.1, and the
base spread is within BAR; it is INCONCLUSIVE when the base spread exceeds
BAR and the candidate is not worse by more than the spread; otherwise it
FAILS.

usage: terminal_latency_live.py --base PATH --candidate PATH --output DIR
                                [--regime loadup|workspace] [--pairs 5]
                                [--keys 250] [--delay 30] [--bar 0.02]
"""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

import fileio_live as fio
import live_oracle as live

READER = r'''import json, os, sys, termios, threading, time, tty
out, mode = sys.argv[1], sys.argv[2]
saved = termios.tcgetattr(0)
stamps, stop = [], threading.Event()
def flood():
    n = 0
    while not stop.is_set():
        os.write(1, ('output line %06d %s\r\n' % (n, 'x' * 60)).encode())
        n += 1
        time.sleep(0.005)
try:
    tty.setraw(0)
    if mode == 'output':
        threading.Thread(target=flood, daemon=True).start()
    open(out + '.ready', 'w').close()
    while True:
        data = os.read(0, 4096)
        stamps.append((time.clock_gettime_ns(time.CLOCK_MONOTONIC), data.hex()))
        if b'\x1d' in data:
            break
finally:
    stop.set()
    termios.tcsetattr(0, termios.TCSANOW, saved)
    with open(out + '.tmp', 'w') as f:
        json.dump(stamps, f)
    os.rename(out + '.tmp', out)
'''


def records(text, name):
    out = []
    for line in text.split('\n'):
        if name in line:
            try:
                out.append(json.loads(line))
            except ValueError:
                pass
    return out


def last_presentation(text):
    numbers = [r['presentation'] for r in records(text, '"presentation"') if isinstance(r.get('presentation'), int)]
    return max(numbers) if numbers else 0


def percentile(values, p):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(p * (len(ordered) - 1))))]


def session(binary, regime, keys, delay, timeout, log_path):
    """One session's phases: {phase: {'latency_ns': [...], 'frames_per_key': F}}."""
    letters = ''.join(chr(ord('a') + i % 26) for i in range(keys))
    with tempfile.TemporaryDirectory(prefix='term-lat-') as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        home, work, config = root / 'home', root / 'work', root / 'config'
        for directory in (home, work, config):
            directory.mkdir()
        (work / 'reader.py').write_text(READER)
        original = work / 'original.txt'
        original.write_text('Terminal latency sentinel\n')
        (config / 'rc.xml').write_text(live.RC_XML)
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
               'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
               'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
               'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '20', 'SHELL': '/bin/sh'}
        compositor = process = keyboard = None
        try:
            with log_path.open('w') as log:
                compositor = subprocess.Popen([live.LABWC, '-C', str(config)], env=env,
                                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), timeout, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([live.RANDR, '--output', 'HEADLESS-1', '--custom-mode', '1600x1000'],
                               env=env, check=True, timeout=15)
                # A keyboard on the seat before the first modifier is sent.
                keyboard = subprocess.Popen(['/usr/bin/wtype', '-s', '600000', '-k', 'Shift_L'], env=env,
                                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                time.sleep(.3)
                argv = [str(binary), '-Q'] + (['--loadup'] if regime == 'loadup' else []) + ['--trace-latency', str(original)]
                process = subprocess.Popen(argv, env=env, cwd=work, stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def trace():
                    return log_path.read_text(errors='replace')

                def type_keys(*parts, pause=.15):
                    subprocess.run(['/usr/bin/wtype', *parts], env=env, check=True, timeout=600)
                    time.sleep(pause)

                # No scene dumps (they would cost every paint): the first
                # paint, then the M-x prompt's, then the reader's own file.
                fio.wait(lambda: '"diagnostic":"paint"' in trace(), timeout, 'first paint', process)
                time.sleep(1)
                before = len(trace())
                type_keys('-M', 'alt', 'x', '-m', 'alt')
                try:
                    fio.wait(lambda: '"prompt-paint"' in trace()[before:], 5, 'the M-x prompt', process)
                except AssertionError:
                    pass  # GNU's minibuffer (--loadup) paints no prompt of the GUI's own
                type_keys('romacs-term', '-k', 'Return', pause=2)
                phases = {}
                for phase in ('typing', 'output'):
                    out = work / f'{phase}.json'
                    type_keys(f'python3 reader.py {phase}.json {phase}', '-k', 'Return')
                    fio.wait(lambda: Path(str(out) + '.ready').exists(), timeout, f'{phase} reader', process)
                    time.sleep(.5)
                    start = trace()
                    offset, first_frame = len(start), last_presentation(start)
                    type_keys('-d', str(delay), letters, pause=.5)
                    end = trace()
                    frames = last_presentation(end) - first_frame
                    type_keys('-M', 'ctrl', ']', '-m', 'ctrl')
                    fio.wait(out.exists, timeout, f'{phase} stamps', process)
                    ingress = [r['monotonic_ns'] for r in records(end[offset:], '"key-ingress"')][:keys]
                    arrivals = []
                    for stamp, data in json.loads(out.read_text()):
                        arrivals.extend((stamp, byte) for byte in bytes.fromhex(data))
                    got = bytes(b for _, b in arrivals[:keys]).decode(errors='replace')
                    if got != letters or len(ingress) != keys:
                        phases[phase] = {'refused': f'{len(ingress)} ingress records; bytes {got[:40]!r}...'}
                        continue
                    latency = [t - i for (t, _), i in zip(arrivals, ingress)]
                    if min(latency) <= 0:
                        phases[phase] = {'refused': f'a byte arrived before its key: {min(latency)} ns'}
                        continue
                    phases[phase] = {'latency_ns': latency, 'frames_per_key': frames / keys,
                                     'p50_ns': percentile(latency, .5), 'p95_ns': percentile(latency, .95)}
                return phases
        finally:
            for proc in (process, keyboard, compositor):
                if proc is not None:
                    live.stop(proc)


def verdict(base, candidate, bar):
    """PASS, INCONCLUSIVE or FAIL, with the numbers behind it."""
    def median(sessions, key):
        return statistics.median(s[key] for s in sessions)
    b50, c50, b95, c95 = median(base, 'p50_ns'), median(candidate, 'p50_ns'), median(base, 'p95_ns'), median(candidate, 'p95_ns')
    bf, cf = median(base, 'frames_per_key'), median(candidate, 'frames_per_key')
    spread = (max(s['p50_ns'] for s in base) - min(s['p50_ns'] for s in base)) / b50
    d50, d95 = c50 / b50 - 1, c95 / b95 - 1
    numbers = {'base_p50_ns': b50, 'candidate_p50_ns': c50, 'base_p95_ns': b95, 'candidate_p95_ns': c95,
               'p50_delta': d50, 'p95_delta': d95, 'base_frames_per_key': bf, 'candidate_frames_per_key': cf,
               'base_spread': spread,
               'pair_p50_deltas': [c['p50_ns'] / b['p50_ns'] - 1 for b, c in zip(base, candidate)]}
    frames_ok = cf - bf < 0.1
    if spread > bar:
        # The base varies more than the bar: no pass is possible.
        result = 'INCONCLUSIVE' if frames_ok and d50 <= spread and d95 <= spread else 'FAIL'
    elif d50 <= bar and d95 <= bar and frames_ok:
        result = 'PASS'
    else:
        result = 'FAIL'
    return result, numbers


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--base', required=True, type=Path)
    parser.add_argument('--candidate', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--regime', choices=['loadup', 'workspace'], default='loadup')
    parser.add_argument('--pairs', type=int, default=5)
    parser.add_argument('--keys', type=int, default=250)
    parser.add_argument('--delay', type=int, default=30)
    parser.add_argument('--bar', type=float, default=0.02)
    parser.add_argument('--timeout', type=float, default=60)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    runs = {'base': [], 'candidate': []}
    for pair in range(args.pairs):
        order = ('base', 'candidate') if pair % 2 == 0 else ('candidate', 'base')
        for who in order:
            binary = args.base if who == 'base' else args.candidate
            phases = session(binary.resolve(), args.regime, args.keys, args.delay, args.timeout,
                             args.output / f'{who}-{pair}.stderr')
            runs[who].append(phases)
            print(who, pair, {p: v.get('refused') or (v['p50_ns'], v['p95_ns'], round(v['frames_per_key'], 3)) for p, v in phases.items()}, flush=True)
    report = {'regime': args.regime, 'pairs': args.pairs, 'keys': args.keys, 'delay_ms': args.delay, 'bar': args.bar,
              'span': 'host key enqueue (key-ingress, CLOCK_MONOTONIC) to the byte read in the pty (raw-mode reader, CLOCK_MONOTONIC)',
              'phases': {}}
    for phase in ('typing', 'output'):
        base = [s[phase] for s in runs['base'] if 'latency_ns' in s.get(phase, {})]
        candidate = [s[phase] for s in runs['candidate'] if 'latency_ns' in s.get(phase, {})]
        if len(base) < args.pairs or len(candidate) < args.pairs:
            report['phases'][phase] = {'result': 'REFUSED', 'measured': [len(base), len(candidate)]}
            continue
        result, numbers = verdict(base, candidate, args.bar)
        report['phases'][phase] = {'result': result, **numbers}
    for who, sessions in runs.items():
        for s in sessions:
            for v in s.values():
                v.pop('latency_ns', None)
    report['sessions'] = runs
    (args.output / 'report.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report['phases'], indent=2))
    return 0 if all(p['result'] == 'PASS' for p in report['phases'].values()) else 1


if __name__ == '__main__':
    raise SystemExit(main())
