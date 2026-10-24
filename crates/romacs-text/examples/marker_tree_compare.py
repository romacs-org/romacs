#!/usr/bin/env python3
"""Run already-built P2 binaries; fail checksum mismatches and absolute gates.

Run under neomacs-slot props --group props --light -- bash SCRIPTFILE.
The array reference maintains BOTH byte and char positions; this is not a
like-for-like char-only tree or GNU Emacs benchmark. No compilation here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys


def sample(binary, backend, count, workload, prefix, perf):
    ctl, ack = str(prefix) + '.ctl', str(prefix) + '.ack'
    os.mkfifo(ctl)
    os.mkfifo(ack)
    try:
        env = dict(os.environ, PERF_CTL=ctl, PERF_ACK=ack)
        result = subprocess.run(
            [perf, 'stat', '--delay=-1', '--control=fifo:' + ctl + ',' + ack,
             '-x,', '-e', 'instructions:u,cycles:u,cache-misses:u',
             '-o', str(prefix) + '.perf', '--', binary, backend, str(count), workload],
            env=env, text=True, capture_output=True, check=True)
        Path(str(prefix) + '.txt').write_text(result.stdout)
        fields = dict(item.split('=', 1) for item in result.stdout.strip().split())
        assert fields['backend'] == backend
        assert int(fields['markers']) == count
        assert fields['typing'] == str(workload == 'typing').lower()
        row = {k: float(fields[k]) for k in ('ns_per_edit', 'bytes_per_marker')}
        row.update({k: int(fields[k]) for k in ('checksum', 'edits', 'bytes')})
        events = {}
        for line in Path(str(prefix) + '.perf').read_text().splitlines():
            cells = line.split(',')
            if len(cells) > 2 and cells[2] in ('instructions:u', 'cycles:u', 'cache-misses:u'):
                events[cells[2]] = float(cells[0]) / row['edits']
        assert len(events) == 3, 'Missing perf counters: ' + str(events)
        row.update(events)
        return row
    finally:
        os.unlink(ctl)
        os.unlink(ack)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--reference', help='Defaults to candidate with array backend')
    parser.add_argument('--reference-backend', choices=['array', 'tree'], default='array')
    parser.add_argument('--perf', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--repetitions', type=int, default=5)
    args = parser.parse_args()
    assert args.repetitions >= 3
    args.reference = args.reference or args.candidate
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=False)
    report = {'binaries': {label: {'path': path, 'sha256': hashlib.sha256(Path(path).read_bytes()).hexdigest()}
                           for label, path in [('candidate', args.candidate), ('reference', args.reference)]},
              'reference_backend': args.reference_backend, 'samples': [], 'medians': [], 'failures': []}
    try:
        for workload, count, gate in [('edits', 10000, 1000), ('edits', 100000, 5000),
                                      ('typing', 1000, 60), ('typing', 10000, None),
                                      ('typing', 100000, None)]:
            pairs = []
            for rep in range(args.repetitions):
                pair = {}
                # Alternate first runner to avoid always favoring one backend.
                labels = ['reference', 'candidate'] if rep % 2 == 0 else ['candidate', 'reference']
                for label in labels:
                    backend = args.reference_backend if label == 'reference' else 'tree'
                    binary = args.reference if label == 'reference' else args.candidate
                    pair[label] = sample(binary, backend, count, workload,
                                         output / f'{workload}-{count}-{rep}-{label}', args.perf)
                assert pair['candidate']['checksum'] == pair['reference']['checksum'], pair
                assert pair['candidate']['edits'] == pair['reference']['edits'], pair
                report['samples'].append(dict(workload=workload, markers=count, rep=rep, **pair))
                pairs.append(pair)
            medians = {label: {key: statistics.median(pair[label][key] for pair in pairs)
                               for key in pairs[0][label]} for label in ['candidate', 'reference']}
            row = dict(workload=workload, markers=count, gate_ns=gate, **medians)
            report['medians'].append(row)
            print(json.dumps(row), flush=True)
            if gate is not None and medians['candidate']['ns_per_edit'] > gate:
                report['failures'].append(f'{workload}/{count} median exceeds {gate} ns')
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    if report['failures']:
        print('\n'.join(report['failures']), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
