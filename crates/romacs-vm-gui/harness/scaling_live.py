#!/usr/bin/env python3
"""Scaling probes: each subsystem's work at size N and K*N, and its exponent.

Every probe of scaling/probes.el runs at N and at K*N (K is 4 unless the
probe says otherwise), each in a fresh session, and is measured by the
instructions the evaluating thread retires in its body (`romacs-instructions',
a pinned hardware counter: repeatable and load-insensitive, unlike time).
The empirical exponent is

    log(work(K*N) / work(N)) / log(K)

1 for linear work, about 1.15 for n log n at these sizes, 2 for quadratic.
A probe passes when its exponent is within its budget (default 1.25); a
probe whose super-linear cost is a known, ticketed debt (:known) reports
KNOWN instead of failing.  A body that does under MIN_WORK instructions at
N is refused: its exponent would measure the probe's own overhead.  A
:stack probe also passes only if the native stack room it records at each
`scaling-stack-mark' varies by under STACK_SLACK bytes at K*N: a loader that
keeps a native frame per form fails it long before it exhausts the stack.

Batch probes run as `BINARY -Q --batch -l probes.el --eval (scaling-run ...)'
sessions restored from a batch world prepared once in a private HOME.  GUI
probes (:gui) run as `BINARY -Q --loadup' windows in a private headless
labwc: once idle the session sets the probe up, then runs its R steps from
a chain of timers, each once the host has redisplayed the step before
(`romacs--redisplay-since-mark': a paint in flight holds the next one
back), and counts from before the first step to after the last step's
layout and paint submission; the steps' work includes the redisplay they
cause.  A GUI measurement is refused
unless the host's diagnostics show a paint submitted (laid out by the
evaluator) after every step, before the next step's mark or the end mark.

--record profiles the body of every batch probe over its budget at K*N
with `perf record --call-graph fp', counting only while the probe enables
it, and writes each probe's top inclusive symbols beside the report (as
PROBE.callgraph.txt).  Whole stacks need frame pointers: measure a build
made with RUSTFLAGS='-C force-frame-pointers=yes'.

A batch session is bounded by --timeout seconds of CPU (RLIMIT_CPU), which
a loaded host does not use up the way it stretches wall time; the wall
clock, at ten times that, only ends a session that waits without working.

Writes OUTPUT/report.json (status PASS or FAIL) and OUTPUT/table.md, and
prints the table.

usage: scaling_live.py --binary PATH --output DIR [--only NAME ...]
                       [--mode all|batch|gui] [--jobs 4] [--record]
                       [--timeout 300]
"""
import argparse
import concurrent.futures
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
PROBES = HERE / 'scaling' / 'probes.el'
MIN_WORK = 1_000_000
STACK_SLACK = 64 * 1024
DEFAULT_BUDGET = 1.25
# A session that spends no CPU for this many times its CPU bound is hung.
WALL_FACTOR = 10


class Refused(Exception):
    """A session or measurement that must not become a number."""


# Pure parts: exponents, verdicts and the table.

def exponent(work_n, work_kn, k):
    """The empirical exponent of WORK_N at N growing to WORK_KN at K*N."""
    return math.log(work_kn / work_n) / math.log(k)


def stack_growth(marks):
    """The spread of a :stack probe's room marks, in bytes (0 without marks)."""
    return max(marks) - min(marks) if marks else 0


def verdict(probe, small, large):
    """PROBE's row from its N and K*N results: status PASS, KNOWN or FAIL."""
    row = {'name': probe['name'], 'n': probe['n'], 'k': probe['k'], 'budget': probe['budget'],
           'known': probe.get('known'), 'gui': probe['gui']}
    work_n, work_kn = small.get('instructions'), large.get('instructions')
    if work_n is None or work_kn is None:
        return {**row, 'status': 'FAIL', 'reason': 'no instruction counter (romacs-instructions is nil)'}
    row.update(work_n=work_n, work_kn=work_kn)
    if work_n < MIN_WORK:
        return {**row, 'status': 'FAIL', 'reason': f'body does {work_n} instructions at N, under {MIN_WORK}'}
    if work_kn <= 0:
        return {**row, 'status': 'FAIL', 'reason': f'body does {work_kn} instructions at K*N'}
    row['exponent'] = round(exponent(work_n, work_kn, probe['k']), 3)
    problems = []
    if row['exponent'] > probe['budget']:
        problems.append(f"exponent {row['exponent']} over budget {probe['budget']}")
    if probe.get('stack'):
        row['stack_growth'] = stack_growth(large.get('stack') or [])
        if not large.get('stack'):
            problems.append('no stack marks at K*N')
        elif row['stack_growth'] > STACK_SLACK:
            problems.append(f"native stack room varies by {row['stack_growth']} bytes over the body")
    if not problems:
        stale = {'reason': 'within budget: its known override can go'} if probe.get('known') else {}
        return {**row, 'status': 'PASS', **stale}
    status = 'KNOWN' if probe.get('known') and all(p.startswith('exponent') for p in problems) else 'FAIL'
    return {**row, 'status': status, 'reason': '; '.join(problems)}


def overall(rows):
    """PASS unless a row failed or a selected probe has no row."""
    return 'PASS' if rows and all(r['status'] in ('PASS', 'KNOWN') for r in rows) else 'FAIL'


def table(rows):
    """The report as a Markdown table."""
    lines = ['| probe | N | k | work N | work kN | exponent | budget | status |',
             '|---|---|---|---|---|---|---|---|']
    for r in rows:
        work = lambda w: f'{w / 1e6:.1f}M' if isinstance(w, int) else '-'  # noqa: E731
        note = r.get('reason') or ''
        if r.get('known'):
            note = f"{note}; known: {r['known']}" if note else f"known: {r['known']}"
        if 'stack_growth' in r:
            note = f"{note}; stack spread {r['stack_growth']} B" if note else f"stack spread {r['stack_growth']} B"
        lines.append(f"| {r['name']}{' (gui)' if r.get('gui') else ''} | {r['n']} | {r['k']} | {work(r.get('work_n'))}"
                     f" | {work(r.get('work_kn'))} | {r.get('exponent', '-')} | {r['budget']} | {r['status']}"
                     f"{' — ' + note if note else ''} |")
    return '\n'.join(lines) + '\n'


def scaling_lines(text):
    """The JSON objects of TEXT's `SCALING ' lines."""
    return [json.loads(line[len('SCALING '):]) for line in text.splitlines() if line.startswith('SCALING ')]


def step_paints(stderr_text):
    """Paints submitted after each step of a GUI probe: between its step mark
    and the next step mark, or the end mark for the last step.  Submission is
    the evaluator's layout of the step, the work counted; the render
    thread's completion (`paint') may land later under load."""
    counts, current = [], None
    for line in stderr_text.splitlines():
        if not line.startswith('{'):
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        mark = record.get('scaling')
        if mark == 'step':
            counts.append(0)
            current = len(counts) - 1
        elif mark in ('start', 'end'):
            current = None
        elif current is not None and record.get('diagnostic') == 'paint-submitted':
            counts[current] += 1
    return counts


def probe_entry(raw):
    """A probe from `scaling-list''s JSON, nulls and defaults normalised."""
    return {'name': raw['name'], 'n': int(raw['n']), 'k': int(raw.get('k') or 4),
            'budget': float(raw.get('budget') or DEFAULT_BUDGET), 'known': raw.get('known') or None,
            'gui': raw.get('gui') is True, 'steps': int(raw.get('steps') or 1), 'stack': raw.get('stack') is True}


# Sessions.

def private_env(home, extra=None):
    """A private HOME/XDG/TMPDIR environment with no display. The collector marks
    on the evaluating thread alone, so that thread's instructions are all the work."""
    env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_GC_WORKERS': '1',
           'XDG_CONFIG_HOME': str(home / 'config'), 'XDG_CACHE_HOME': str(home / 'cache'),
           'XDG_DATA_HOME': str(home / 'data'), 'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(home / 'tmp')}
    for name in ('ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR', 'ROMACS_GNU_SOURCE', 'ROMACS_GATE_FONTCONFIG'):
        if name in os.environ:
            env[name] = os.environ[name]
    if 'ROMACS_GATE_FONTCONFIG' in env:
        env['FONTCONFIG_FILE'] = env['ROMACS_GATE_FONTCONFIG']
    return {**env, **(extra or {})}


def bounded(what, argv, timeout, **kwargs):
    """ARGV run to its end within TIMEOUT seconds of CPU, its output captured;
    a session that waits without working ends at WALL_FACTOR times that."""
    cpu = max(1, math.ceil(timeout))
    # prlimit, not a preexec_fn: callers run this from a thread pool.
    try:
        done = subprocess.run(['prlimit', f'--cpu={cpu}:{cpu + 1}', '--', *argv], capture_output=True, text=True,
                              timeout=timeout * WALL_FACTOR, **kwargs)
    except subprocess.TimeoutExpired:
        raise Refused(f'{what}: no end after {timeout * WALL_FACTOR:.0f}s, within {cpu}s of CPU: hung') from None
    if done.returncode == -signal.SIGXCPU:
        raise Refused(f'{what}: used its {cpu}s of CPU')
    if done.returncode == -signal.SIGKILL:
        raise Refused(f'{what}: killed (SIGKILL) under a CPU limit of {cpu}s, hard {cpu + 1}s')
    return done


def batch(binary, home, form, timeout, prefix=(), env=None):
    """Run FORM after loading the probes in a batch session; its SCALING lines."""
    argv = [*prefix, str(binary), '-Q', '--batch', '-l', str(PROBES), '--eval', form]
    done = bounded(form, argv, timeout, env=private_env(home, env), cwd=home)
    lines = scaling_lines(done.stdout)
    if done.returncode != 0 or not lines:
        raise Refused(f'{form}: exit {done.returncode}: {done.stderr.strip()[-600:]}')
    return lines


def prepare(binary, home, timeout):
    """Build the GUI and batch worlds in HOME, as a first session does."""
    for sub in ('config', 'cache', 'data', 'state', 'tmp'):
        (home / sub).mkdir(parents=True, exist_ok=True)
    built = bounded('--world-build', [str(binary), '--loadup', '--world-build'], timeout, env=private_env(home), cwd=home)
    if built.returncode != 0:
        raise Refused(f'--world-build: exit {built.returncode}: {built.stderr.strip()[-600:]}')
    probes = [probe_entry(raw) for raw in batch(binary, home, '(scaling-list)', timeout)]
    store = home / 'cache' / 'romacs' / 'worlds'
    deadline = time.monotonic() + timeout
    while sum(1 for image in store.glob('*.rwi') if image.with_suffix('.json').is_file()) < 2:
        if time.monotonic() > deadline:
            raise Refused('no batch world after the first session')
        time.sleep(.2)
    return probes


def run_batch_probe(binary, home, probe, timeout):
    """PROBE's row from batch sessions at N and K*N."""
    small, large = (batch(binary, home, f"(scaling-run '{probe['name']} {n})", timeout)[-1]
                    for n in (probe['n'], probe['n'] * probe['k']))
    return verdict(probe, small, large)


class Gui:
    """A private headless labwc for --wayland sessions."""

    def __init__(self, home, runtime, timeout):
        import live_oracle as live
        self.live, self.home, self.runtime, self.timeout = live, home, runtime, timeout
        (runtime / 'config').mkdir()
        (runtime / 'config' / 'rc.xml').write_text(live.RC_XML)
        self.env = private_env(home, {'XDG_RUNTIME_DIR': str(runtime), 'ROMACS_DIAGNOSTICS': '1',
                                      'WLR_BACKENDS': 'headless', 'WLR_HEADLESS_OUTPUTS': '1',
                                      'WLR_RENDERER': 'pixman', 'WLR_LIBINPUT_NO_DEVICES': '1',
                                      'LIBSEAT_BACKEND': 'noop'})
        labwc = shutil.which(os.environ.get('ROMACS_TEST_LABWC', 'labwc'), path=self.env['PATH'])
        self.log = (home / 'compositor.log').open('w')
        self.compositor = subprocess.Popen([labwc or 'labwc', '-C', str(runtime / 'config')], env=self.env,
                                           stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + timeout
        while not any(p.is_socket() for p in runtime.glob('wayland-*')):
            if self.compositor.poll() is not None or time.monotonic() > deadline:
                raise Refused('private compositor failed')
            time.sleep(.05)
        self.socket = next(p for p in runtime.glob('wayland-*') if p.is_socket())
        self.env['WAYLAND_DISPLAY'] = self.socket.name
        randr = shutil.which(os.environ.get('ROMACS_TEST_WLR_RANDR', 'wlr-randr'), path=self.env['PATH'])
        subprocess.run([randr or 'wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1600x1000'],
                       env=self.env, check=True, timeout=15, stdout=subprocess.DEVNULL)

    def measure(self, probe, n):
        """The steps' instructions of PROBE at N in a fresh --loadup session,
        refused unless the host submitted a paint after every step."""
        out = self.home / 'tmp' / f"{probe['name']}-{n}.json"
        stderr = self.home / f"{probe['name']}-{n}.stderr"
        form = f'(scaling-gui-start (quote {probe["name"]}) {n} "{out}")'
        # Append mode: the probe's marks append to the same file.
        with stderr.open('a') as log:
            proc = subprocess.Popen([str(self.binary), '-Q', '--loadup', '-l', str(PROBES), '--eval', form],
                                    env=self.env, cwd=self.home, stdin=subprocess.DEVNULL,
                                    stdout=subprocess.DEVNULL, stderr=log)
            try:
                proc.wait(timeout=self.timeout / 5)
            except subprocess.TimeoutExpired:
                raise Refused(f"{probe['name']} at {n}: GUI session timed out") from None
            finally:
                self.live.stop(proc)
        if proc.returncode != 0 or not out.is_file():
            raise Refused(f"{probe['name']} at {n}: exit {proc.returncode}: {stderr.read_text(errors='replace')[-600:]}")
        paints = step_paints(stderr.read_text(errors='replace'))
        if len(paints) != probe['steps'] or 0 in paints:
            raise Refused(f"{probe['name']} at {n}: paints after each of {probe['steps']} steps: {paints}")
        return json.loads(out.read_text())

    def close(self):
        self.live.stop(self.compositor)
        self.log.close()


def run_gui_probes(binary, home, probes, timeout):
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    rows = []
    with tempfile.TemporaryDirectory(prefix='scaling-', dir=runtime) as temporary:
        Path(temporary).chmod(0o700)
        gui = Gui(home, Path(temporary), timeout)
        gui.binary = binary
        try:
            for probe in probes:
                try:
                    rows.append(verdict(probe, *(gui.measure(probe, n) for n in (probe['n'], probe['n'] * probe['k']))))
                except (Refused, RuntimeError, OSError, ValueError) as error:
                    rows.append(refused(probe, error))
        finally:
            gui.close()
    return rows


def refused(probe, error):
    return {'name': probe['name'], 'n': probe['n'], 'k': probe['k'], 'budget': probe['budget'],
            'known': probe.get('known'), 'gui': probe['gui'], 'status': 'FAIL', 'reason': str(error)}


def record(binary, home, row, output, timeout):
    """A call-graph profile of ROW's probe body at K*N, its top inclusive symbols.
    perf starts disabled; the probe enables it around the body alone."""
    data = output / f"{row['name']}.perf.data"
    fifo = home / 'tmp' / f"{row['name']}.perf-ctl"
    os.mkfifo(fifo)
    form = f"(scaling-run '{row['name']} {row['n'] * row['k']})"
    try:
        batch(binary, home, form, timeout, env={'SCALING_PERF_CTL': str(fifo)},
              prefix=('perf', 'record', '-q', '-D', '-1', '--control', f'fifo:{fifo}', '-e', 'instructions:u',
                      '--call-graph', 'fp', '-o', str(data), '--'))
        report = subprocess.run(['perf', 'report', '-i', str(data), '--stdio', '--children', '--percent-limit', '3',
                                 '--sort', 'symbol', '-g', 'none'], capture_output=True, text=True, timeout=timeout)
    except (Refused, subprocess.TimeoutExpired, OSError) as error:
        return f'record failed: {error}'
    finally:
        fifo.unlink(missing_ok=True)
    text = '\n'.join(' '.join(line.split()) for line in report.stdout.splitlines()
                     if line.strip() and not line.startswith('#'))
    (output / f"{row['name']}.callgraph.txt").write_text(text + '\n')
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--only', nargs='*', default=None)
    parser.add_argument('--mode', choices=('all', 'batch', 'gui'), default='all')
    parser.add_argument('--jobs', type=int, default=4)
    parser.add_argument('--record', action='store_true')
    parser.add_argument('--timeout', type=float, default=300, help='CPU seconds per batch session')
    args = parser.parse_args()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True)
    binary = args.binary.resolve()
    home = args.output / 'home'
    report = {'binary': str(binary), 'probes_file': str(PROBES), 'status': 'FAIL', 'rows': []}
    started = time.monotonic()
    try:
        probes = prepare(binary, home, args.timeout)
        if args.only is not None:
            unknown = set(args.only) - {p['name'] for p in probes}
            if unknown:
                raise Refused(f'no such probes: {sorted(unknown)}')
            probes = [p for p in probes if p['name'] in args.only]
        batch_probes = [p for p in probes if not p['gui'] and args.mode != 'gui']
        gui_probes = [p for p in probes if p['gui'] and args.mode != 'batch']
        rows = {}
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = {pool.submit(run_batch_probe, binary, home, p, args.timeout): p for p in batch_probes}
            for future, probe in futures.items():
                try:
                    rows[probe['name']] = future.result()
                except Refused as error:
                    rows[probe['name']] = refused(probe, error)
        if gui_probes:
            for row in run_gui_probes(binary, home, gui_probes, args.timeout):
                rows[row['name']] = row
        report['rows'] = [rows[p['name']] for p in batch_probes + gui_probes]
        if args.record:
            for row in report['rows']:
                if row['status'] != 'PASS' and not row.get('gui') and 'exponent' in row:
                    row['callgraph'] = record(binary, home, row, args.output, args.timeout)
        report['status'] = overall(report['rows'])
    except (Refused, subprocess.SubprocessError, OSError) as error:
        report['error'] = str(error)
    report['seconds'] = round(time.monotonic() - started, 1)
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    text = table(report['rows'])
    (args.output / 'table.md').write_text(text)
    print(text, end='')
    print(f"{report['status']} in {report['seconds']}s{': ' + report['error'] if 'error' in report else ''}")
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
