#!/usr/bin/env python3
"""Compare P0 typed GNU oracles with a startup-image-verified Romacs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / 'test/gnudiff/props'


def rows(text):
    lines = text.splitlines()
    if not lines or lines[0] != 'gnudiff-1':
        raise ValueError('Missing gnudiff transport header')
    result = {}
    for line in lines[1:]:
        fields = line.split('\t')
        name = bytes.fromhex(fields[1]).decode()
        if fields[0] != 'case' or name in result:
            raise ValueError('Malformed or duplicate oracle case')
        result[name] = line
    return result


def run(binary, files, env):
    proc = subprocess.run([binary, '-Q', '--batch', '-l',
                           str(ROOT / 'crates/romacs-gnudiff/lisp/gnudiff.el'),
                           *map(str, files)], env=env, capture_output=True,
                          text=True, timeout=300)
    if proc.returncode:
        raise ValueError(f'{binary}: exit {proc.returncode}: {proc.stderr[-2000:]}')
    return rows(proc.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gnu', default='/usr/bin/emacs')
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--xdisp', required=True)
    parser.add_argument('--json', required=True)
    args = parser.parse_args()
    env = dict(os.environ, CORE_BENCH_XDISP=str(Path(args.xdisp).resolve()))
    probe = subprocess.run([args.romacs, '-Q', '--batch', '--eval',
                            '(unless (and (fboundp (quote pdumper-stats)) (eq (cdr (assq (quote dumped-with-pdumper) (pdumper-stats))) t)) (error "Startup image required"))',
                            '--eval', '(prin1 (pdumper-stats))'],
                           env=env, capture_output=True, text=True, timeout=60)
    if probe.returncode or 'dumped-with-pdumper . t' not in probe.stdout:
        raise ValueError(f'Startup image verification failed: {probe.stdout} {probe.stderr}')
    expected = rows((FIXTURES / 'expected.tsv').read_text())
    expected.update(rows((FIXTURES / 'expected-fontlock.tsv').read_text()))
    files = sorted(FIXTURES.glob('[0-9][0-9]-*.el'))
    gnu = run(args.gnu, files, env)
    romacs = run(args.romacs, files, env)
    if set(expected) != set(gnu) or set(expected) != set(romacs):
        raise ValueError('Oracle case set differs')
    results = {name: {'gnu_record_equal': gnu[name] == value,
                      'romacs_record_equal': romacs[name] == value}
               for name, value in expected.items()}
    output = {'pdumper_stats': probe.stdout, 'cases': results,
              'xdisp_sha256': hashlib.sha256(Path(args.xdisp).read_bytes()).hexdigest(),
              'romacs_sha256': hashlib.sha256(Path(args.romacs).read_bytes()).hexdigest()}
    Path(args.json).write_text(json.dumps(output, indent=2) + '\n')
    print(json.dumps(output, indent=2))
    return int(any(not all(result.values()) for result in results.values()))


if __name__ == '__main__':
    raise SystemExit(main())
