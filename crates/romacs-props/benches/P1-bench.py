#!/usr/bin/env python3
"""Prepare standalone A/B crates or run frozen binaries; never builds by itself."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import tarfile
import time
import tomllib

PREFIX = Path(os.environ.get('P1_SCRATCH_ROOT', str(Path(os.environ.get('TMPDIR', Path.home() / '.cache')) / 'props-bench'))).absolute()
ROWS = ('P1', 'P2', 'P3', 'P4', 'P5')
# Pinned baseline: the archived pre-P1 Props implementation. Never HEAD, which
# moves as unrelated commits land and would silently change the old arm.
BASELINE_COMMIT = '1a5418fcaf375ecb873bda6c94d1b14389815cf4'
# Binding absolute acceptance gates: candidate median wall ns/op must be <= limit.
WALL_GATES_NS = {'P1': 20.0, 'P4': 80.0, 'P5': 10.0}
WALL_GATE_LABELS = {'P1': 'append', 'P4': 'random', 'P5': 'cursor/next_single scan'}
MEMORY_GATE_B_PER_RUN = 30.25


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def owned_root(value):
    root = Path(value).absolute()
    if root.parent != PREFIX or not root.name.startswith('P1-bench'):
        raise ValueError(f'output must be directly under {PREFIX}/P1-bench*')
    if root.is_symlink():
        raise ValueError('output must not be a symlink')
    return root


def standalone_manifest(path, chars):
    # Keep library name/API; omit dev-dependencies, targets and workspace
    # inheritance so neither arm resolves the heavyweight repository workspace.
    original = tomllib.loads(path.read_text())
    package = original['package']
    dependencies = original.get('dependencies', {})
    unexpected = set(dependencies) - {'hashbrown', 'romacs-chars'}
    if unexpected:
        raise ValueError(f'unhandled dependencies: {unexpected}')
    text = (f'[package]\nname = "romacs-props"\nversion = {json.dumps(package["version"])}\n'
            'edition = "2024"\nlicense = "GPL-3.0-or-later"\n'
            '[workspace]\n[lints.rust]\nunsafe_code = "forbid"\n'
            '[dependencies]\nromacs-chars = { path = ' + json.dumps(str(chars)) + ' }\n')
    if 'hashbrown' in dependencies:
        locked=tomllib.loads((chars.parents[1]/'Cargo.lock').read_text())
        version=next(p['version'] for p in locked['package'] if p['name']=='hashbrown' and p['version'].startswith('0.17.'))
        text += f'hashbrown = "={version}"\n'
    path.write_text(text)


def prepare(args):
    root = owned_root(args.root)
    root.mkdir(parents=True, exist_ok=False)
    repo = Path(args.repo).resolve()
    chars = (repo / 'crates/romacs-chars').resolve(strict=True)
    head = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', 'HEAD'], text=True).strip()
    baseline = subprocess.check_output(
        ['git', '-C', str(repo), 'rev-parse', '--verify', BASELINE_COMMIT + '^{commit}'], text=True).strip()
    if baseline != BASELINE_COMMIT:
        raise RuntimeError(f'baseline resolved to {baseline}, expected {BASELINE_COMMIT}')
    archive = subprocess.check_output(['git', '-C', str(repo), 'archive', baseline, 'crates/romacs-props'])
    (root / 'baseline-props.tar').write_bytes(archive)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tf:
        tf.extractall(root / 'original-archive', filter='data')
    old = root / 'original-archive/crates/romacs-props'
    with (old/'src/leaves.rs').open('a') as out:
        out.write('''
impl Leaves {
    pub(crate) fn p1_allocated_bytes(&self)->usize {
        fn count(link:&Link)->usize {link.as_ref().map_or(0,|n|std::mem::size_of::<Node>()+n.chunk.runs.capacity()*std::mem::size_of::<super::Run>()+count(&n.left)+count(&n.right))}
        count(&self.root)
    }
}
''')
    with (old/'src/lib.rs').open('a') as out:
        out.write('''
impl Props {
    pub fn p1_allocated_bytes(&self)->usize {std::mem::size_of::<Self>()+self.leaves.p1_allocated_bytes()}
}
''')
    candidate = root / 'candidate-snapshot'
    candidate.mkdir()
    shutil.copytree(repo / 'crates/romacs-props/src', candidate / 'src')
    shutil.copy2(repo / 'crates/romacs-props/Cargo.toml', candidate / 'Cargo.toml')
    harness = Path(__file__).with_name('P1-bench.rs')
    (root / 'harness.rs').write_bytes(harness.read_bytes())
    source_hashes = {}
    for arm, library in [('old', old), ('candidate', candidate)]:
        source_hashes[arm] = {str(p.relative_to(library)): sha(p)
                              for p in sorted(library.rglob('*')) if p.is_file()}
        standalone_manifest(library / 'Cargo.toml', chars)
        driver = root / arm
        (driver / 'src').mkdir(parents=True)
        shutil.copy2(root / 'harness.rs', driver / 'src/main.rs')
        (driver / 'Cargo.toml').write_text(
            '[package]\nname = "props-p1-driver"\nversion = "0.0.0"\nedition = "2024"\n'
            '[workspace]\n[dependencies]\nromacs-props = { path = ' + json.dumps(str(library)) + ' }\n'
            'romacs-chars = { path = ' + json.dumps(str(chars)) + ' }\n'
            '[profile.release]\nopt-level = 3\nlto = "off"\ncodegen-units = 1\ndebug = 1\n'
            'debug-assertions = false\noverflow-checks = false\n')
    status = subprocess.check_output(['git', '-C', str(repo), 'status', '--porcelain'], text=True)
    write_json(root / 'provenance.json', {
        'baseline_commit': baseline, 'candidate_head': head, 'repo': str(repo), 'git_status_at_prepare': status,
        'harness_sha256': sha(root / 'harness.rs'), 'source_hashes_before_manifest_rewrite': source_hashes,
        'chars_path': str(chars), 'chars_source_hashes': {
            str(p.relative_to(chars)): sha(p) for p in sorted((chars / 'src').rglob('*')) if p.is_file()},
        'note': f'Original library is git archive of pinned baseline {baseline} (not HEAD); candidate src is a dirty-working-tree snapshot. '
                'romacs-chars uses the same canonical live path for both arms; keep it unchanged during builds.'})
    print(root)


def execute(binary, row, directory, perf):
    env = os.environ.copy()
    env.pop('P1_PERF_CONTROL', None)
    env.pop('P1_PERF_ACK', None)
    command = [str(binary), row]
    if perf:
        control, ack = directory / 'control.fifo', directory / 'ack.fifo'
        os.mkfifo(control)
        os.mkfifo(ack)
        # perf opens these O_RDWR; matching Rust opens happen after setup.
        env.update(P1_PERF_CONTROL=str(control), P1_PERF_ACK=str(ack))
        command = [perf, 'stat', '--no-big-num', '-x', ';', '-e', 'instructions:u',
                   '--delay=-1', '--control', f'fifo:{control},{ack}',
                   '-o', str(directory / 'perf.csv'), '--'] + command
    started = time.time()
    try:
        process = subprocess.run(command, env=env, text=True, capture_output=True, timeout=1800)
    except subprocess.TimeoutExpired as exc:
        (directory / 'error.txt').write_text(str(exc))
        raise RuntimeError(f'benchmark timeout: {directory}') from exc
    (directory / 'stdout.txt').write_text(process.stdout)
    (directory / 'stderr.txt').write_text(process.stderr)
    if process.returncode:
        raise RuntimeError(f'{command}: exit {process.returncode}: {process.stderr}')
    result = json.loads(process.stdout.strip())
    result['process_start_unix'] = started
    result['instructions'] = None
    if perf:
        for line in (directory / 'perf.csv').read_text().splitlines():
            fields = line.split(';')
            if len(fields) >= 3 and fields[2].strip() == 'instructions:u':
                try:
                    result['instructions'] = int(fields[0].strip())
                except ValueError as exc:
                    raise RuntimeError(f'perf instructions unavailable: {line}') from exc
                # A multiplexed/scaled counter is not an exact instruction count.
                if len(fields) > 4 and fields[4].strip():
                    pct = float(fields[4].strip().rstrip('%'))
                    if pct < 99.0:
                        raise RuntimeError(f'perf counter ran for only {pct}%: {line}')
        if result['instructions'] is None:
            raise RuntimeError(f'perf returned no instruction count: {directory}')
    return result


def run(args):
    root = owned_root(args.root)
    if args.reps < 5:
        raise ValueError('at least five interleaved repetitions required')
    output = root / ('results-' + time.strftime('%Y%m%d-%H%M%S'))
    output.mkdir(exist_ok=False)
    binaries = {arm: root / 'frozen' / arm for arm in ('old', 'candidate')}
    hashes = {arm: sha(binary) for arm, binary in binaries.items()}
    perf = shutil.which(args.perf) if args.perf != 'off' else None
    perf_status = 'disabled by request' if args.perf == 'off' else 'perf executable not found'
    # Probe only; never include this sample in aggregates. Failure is disclosed,
    # then all measured arms run without perf (not mixed scopes).
    if perf:
        probe = output / 'perf-probe'
        probe.mkdir()
        try:
            execute(binaries['candidate'], 'P1', probe, perf)
            perf_status = 'available: body FIFO gated instructions:u (timestamp/control overhead included)'
        except (RuntimeError, OSError, ValueError) as exc:
            perf_status = 'UNAVAILABLE: ' + str(exc)
            perf = None
    records = []
    expected = {}
    gate_failures = []
    with (output / 'raw.jsonl').open('w') as raw:
        for rep in range(args.reps):
            for row in ROWS:
                for arm in (('old', 'candidate') if rep % 2 == 0 else ('candidate', 'old')):
                    directory = output / f'{rep:02}-{row}-{arm}'
                    directory.mkdir()
                    load = os.getloadavg()
                    value = execute(binaries[arm], row, directory, perf)
                    if value['row'] != row or value['ops'] <= 0 or value['runs'] <= 0:
                        raise RuntimeError(f'invalid row shape: {value}')
                    signature = tuple(value[key] for key in ('ops', 'answer', 'checksum', 'runs'))
                    if row in expected and expected[row] != signature:
                        raise RuntimeError(f'CHECKSUM/WORKLOAD MISMATCH {row}: {expected[row]} != {signature}')
                    expected[row] = signature
                    if not isinstance(value['allocated_bytes'],int):raise RuntimeError('missing allocation telemetry')
                    value['bytes_per_run']=value['allocated_bytes']/value['runs']
                    if arm=='candidate' and value['bytes_per_run']>MEMORY_GATE_B_PER_RUN:
                        gate_failures.append(f'{rep}/{row}: {value["bytes_per_run"]:.6f} B/run > {MEMORY_GATE_B_PER_RUN}')
                    value.update(arm=arm, rep=rep, loadavg=load)
                    records.append(value)
                    raw.write(json.dumps(value) + '\n')
                    raw.flush()
    assert len(records) == args.reps * len(ROWS) * 2
    if hashes != {arm: sha(binary) for arm, binary in binaries.items()}:
        raise RuntimeError('frozen binary changed during run')
    provenance = json.loads((root / 'provenance.json').read_text())
    if provenance.get('baseline_commit') != BASELINE_COMMIT:
        raise RuntimeError(f'prepared baseline {provenance.get("baseline_commit")} != pinned {BASELINE_COMMIT}')
    summary = {'perf_status': perf_status, 'binary_sha256': hashes, 'reps': args.reps,
               'baseline_commit': BASELINE_COMMIT, 'candidate_head': provenance.get('candidate_head'),
               'memory_gate_B_per_run': MEMORY_GATE_B_PER_RUN, 'memory_gate_pass': not gate_failures,
               'memory_gate_failures': gate_failures, 'wall_gates_ns_per_op': WALL_GATES_NS, 'rows': {}}
    wall_gate_failures = []
    wall_gate_lines = []
    lines = ['# Props P1–P5 A/B', '', f'Baseline: git archive {BASELINE_COMMIT}; candidate: working-tree snapshot '
             f'(HEAD {provenance.get("candidate_head")}).', '', perf_status, '',
             'Wall: timed original workload including P1/P2 construction, P3 clone and P1–P3 final run count; '
             'local Props destruction added as a second timed/counted span after digest (possible cache warming). '
             'Fixture/interner setup, final digest, memory measurement and query replay excluded. '
             'P3 op = apply call; P5 op = next_single call including ten terminating None calls. '
             'Memory = Props::allocated_bytes / live run count, including spare capacity and object, excluding shared PropSets. '
             'Baseline allocation telemetry is injected only into the archived benchmark snapshot; original tar is preserved. '
             'Instructions include timestamp/control transport overhead, not fixture/digest.', '',
             '| row | ops | old ns/op median [min,max] | candidate ns/op median [min,max] | paired ratio median | old ins/op | candidate ins/op | old B/run | candidate B/run | checksum |',
             '|---|---:|---:|---:|---:|---:|---:|---:|---:|---|']
    for row in ROWS:
        groups = {arm: [v for v in records if v['row'] == row and v['arm'] == arm]
                  for arm in binaries}
        stats = {}
        for arm, values in groups.items():
            walls = [v['elapsed_ns'] / v['ops'] for v in values]
            ins = [v['instructions'] / v['ops'] for v in values if v['instructions'] is not None]
            stats[arm] = {'ns_per_op_median': statistics.median(walls),
                          'ns_per_op_min': min(walls), 'ns_per_op_max': max(walls),
                          'instructions_per_op_median': statistics.median(ins) if ins else None}
        paired = [c['elapsed_ns'] / b['elapsed_ns']
                  for b, c in zip(groups['old'], groups['candidate'])]
        memory = max(v['bytes_per_run'] for v in groups['candidate'])
        baseline_memory=max(v['bytes_per_run'] for v in groups['old'])
        summary['rows'][row] = dict(stats, paired_ratio_median=statistics.median(paired),
                                   old_bytes_per_run_max=baseline_memory,
                                   candidate_bytes_per_run_max=memory, checksum=expected[row][2], ops=expected[row][0])
        if row in WALL_GATES_NS:
            limit = WALL_GATES_NS[row]
            measured = stats['candidate']['ns_per_op_median']
            passed = measured <= limit
            summary['rows'][row].update(wall_gate_ns_per_op=limit, wall_gate_pass=passed)
            verdict = 'PASS' if passed else 'FAIL'
            wall_gate_lines.append(f'- {row} {WALL_GATE_LABELS[row]}: candidate median {measured:.3f} ns/op '
                                   f'{"<=" if passed else ">"} {limit:g} ns/op: {verdict}')
            if not passed:
                wall_gate_failures.append(f'{row}: {measured:.3f} ns/op > {limit:g}')
        def wall(arm):
            v = stats[arm]
            return f'{v["ns_per_op_median"]:.3f} [{v["ns_per_op_min"]:.3f},{v["ns_per_op_max"]:.3f}]'
        def instructions(arm):
            v = stats[arm]['instructions_per_op_median']
            return f'{v:.3f}' if v is not None else 'unavailable'
        lines.append(f'| {row} | {expected[row][0]} | {wall("old")} | {wall("candidate")} | '
                     f'{statistics.median(paired):.4f} | {instructions("old")} | '
                     f'{instructions("candidate")} | {baseline_memory:.6f} | {memory:.6f} | {expected[row][2]} |')
    summary['wall_gate_pass'] = not wall_gate_failures
    summary['wall_gate_failures'] = wall_gate_failures
    summary['acceptance_pass'] = not gate_failures and not wall_gate_failures
    lines += ['', 'Checksums and workload signatures: MATCH across all arms/repetitions.',
              f'Memory gate (every candidate sample <= {MEMORY_GATE_B_PER_RUN} B/run): '
              + ('FAIL' if gate_failures else 'PASS'), *gate_failures,
              'Absolute wall gates (candidate median ns/op, binding): '
              + ('FAIL' if wall_gate_failures else 'PASS'), *wall_gate_lines,
              'Acceptance: ' + ('PASS' if summary['acceptance_pass'] else 'FAIL')]
    write_json(output / 'summary.json', summary)
    (output / 'summary.md').write_text('\n'.join(lines) + '\n')
    print('\n'.join(lines))
    print(f'Evidence: {output}')
    if not summary['acceptance_pass']:
        raise SystemExit(2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    prep = sub.add_parser('prepare')
    prep.add_argument('--repo', required=True)
    prep.add_argument('--root', required=True)
    bench = sub.add_parser('run')
    bench.add_argument('--root', required=True)
    bench.add_argument('--reps', type=int, default=5)
    bench.add_argument('--perf', default='perf', help='perf executable/path, or off')
    args = parser.parse_args()
    (prepare if args.command == 'prepare' else run)(args)


if __name__ == '__main__':
    main()
