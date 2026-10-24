#!/usr/bin/env python3
"""RSS guardrail and terminal post-collection live-byte accounting.

GNU includes its preloaded Lisp world; VM2 includes only runtime objects (not
compiler IR/register backing). RSS includes the entire process in both cases.
No claim that terminal live bytes equal peak live bytes.
"""
import argparse
import json
import statistics
import subprocess
import tempfile
from pathlib import Path
from typing import Any
from bench import ROOT, SOURCE, GNU, load, read_form, gnu_command


def measure(command, payload, expected):
    with tempfile.NamedTemporaryFile() as f:
        run = subprocess.run(['/usr/bin/time', '-f', '%M', '-o', f.name, '--'] + command,
                             input=payload, capture_output=True, check=True)
        rss = int(Path(f.name).read_text())
    assert run.stdout.decode().strip() == expected, (command, run.stdout, expected)
    stderr = run.stderr.decode().strip()
    stats = json.loads(stderr) if stderr.startswith('{') else {'live_bytes': int(stderr.split('HEAP:')[-1])}
    return dict(peak_rss_kib=rss, stats=stats, stdout=run.stdout.decode(), stderr=stderr)


def gnu_memory(source):
    cmd = gnu_command(source)
    expression = cmd[-1]
    call = expression[len('(princ '):-1]
    cmd[-1] = f'''(let* ((answer {call}) (heap (garbage-collect)))
      (princ answer)
      (princ (format "HEAP:%d" (apply #'+ (mapcar (lambda (r) (* (nth 1 r) (nth 2 r))) heap))) 'external-debugging-output))'''
    return cmd


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--baseline', required=True)
    args = p.parse_args()
    report: dict[str, Any] = dict(note=__doc__, gnu=subprocess.check_output([GNU, '--version'], text=True).splitlines()[0], cases=[], stress=[])
    binary = str(ROOT / 'target/release/romacs-vm')
    for name, n, body in load(SOURCE, 'slopes').CASES:
        if name == 'buf-insert':
            continue
        source = body.replace('N', str(2*n))
        payload = read_form(source)
        expected = subprocess.check_output(gnu_command(source), text=True).strip()
        row: dict[str, Any] = dict(name=name, n=2*n, samples={})
        for engine, cmd, data in [('vm2', [binary, '--heap-stats'], payload), ('gnu', gnu_memory(source), b'')]:
            row['samples'][engine] = [measure(cmd, data, expected) for _ in range(3)]
        row['peak_rss_ratio'] = max(s['peak_rss_kib'] for s in row['samples']['vm2']) / max(s['peak_rss_kib'] for s in row['samples']['gnu'])
        row['live_bytes_ratio'] = statistics.median(s['stats']['live_bytes'] for s in row['samples']['vm2']) / statistics.median(s['stats']['live_bytes'] for s in row['samples']['gnu'])
        assert row['peak_rss_ratio'] < 5, row
        report['cases'].append(row)
        args.output.write_text(json.dumps(report, indent=2)+'\n')
    for n in [1_000_000, 10_000_000]:
        source = f'(let ((i 0)) (while (< i {n}) (cons i nil) (setq i (1+ i))) i)'
        payload = read_form(source)
        report['stress'].append(dict(n=n, gc=measure([binary, '--heap-stats'], payload, str(n)), arena=None))
        # Arena has no heap-stats output. Measure RSS independently below.
        with tempfile.NamedTemporaryFile() as f:
            run = subprocess.run(['/usr/bin/time', '-f', '%M', '-o', f.name, args.baseline], input=payload, capture_output=True, check=True)
            assert run.stdout.decode().strip() == str(n)
            report['stress'][-1]['arena'] = dict(peak_rss_kib=int(Path(f.name).read_text()))
        args.output.write_text(json.dumps(report, indent=2)+'\n')
    report['complete'] = True
    args.output.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
