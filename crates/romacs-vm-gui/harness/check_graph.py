#!/usr/bin/env python3
"""Verify G1a's package-scoped graph and the exact built GUI binary."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def run(command):
    p = subprocess.run(command, capture_output=True, text=True, check=False)
    return {'command': command, 'exit': p.returncode,
            'stdout': p.stdout, 'stderr': p.stderr}


def no_core_symbols(text):
    if 'romacs_core::' in text:
        raise AssertionError('romacs_core:: symbol leaked into GUI')


def check_tree(text, forbidden):
    for package in forbidden:
        if package in text:
            raise AssertionError(f'{package} leaked into dependency tree')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    args = ap.parse_args()
    binary = args.binary.resolve(strict=True)
    records = []
    inverse = run(['cargo', 'tree', '--locked', '--offline', '-p',
                   'romacs-vm-gui', '-e', 'normal', '-i', 'romacs-core'])
    records.append(inverse)
    assert inverse['exit'] != 0 and 'did not match' in inverse['stderr'], inverse
    for package, forbidden in [('romacs-vm-gui', ['romacs-core']),
                               ('romacs-vm', ['romacs-core']),
                               ('romacs-layout', ['romacs-core', 'romacs-vm'])]:
        result = run(['cargo', 'tree', '--locked', '--offline', '-p', package, '-e', 'normal'])
        records.append(result)
        assert result['exit'] == 0, result
        check_tree(result['stdout'], forbidden)
        if package == 'romacs-vm':
            assert 'romacs-layout-input' in result['stdout'], result
    nm = run(['nm', '-C', str(binary)])
    assert nm['exit'] == 0, nm['stderr']
    no_core_symbols(nm['stdout'])
    assert 'romacs_vm::' in nm['stdout'], 'nm must inspect an unstripped VM2 executable'
    # Qualify assertion helpers against the realistic graph/static-symbol leaks
    # which motivated the paired checks. This is later mutant qualification,
    # not a pre-implementation RED claim and not a substitute for Cargo/nm above.
    mutants = [lambda: no_core_symbols('00000 r romacs_core::PDUMP_FINGERPRINT'),
               lambda: check_tree('romacs-display-runtime\n└── romacs-core v0.0.20', ['romacs-core']),
               lambda: check_tree('romacs-layout\n└── romacs-vm v0.0.20', ['romacs-vm'])]
    for mutate in mutants:
        try:
            mutate()
        except AssertionError:
            continue
        raise AssertionError('graph/symbol assertion mutant survived')
    result = {'pass': True, 'binary': str(binary),
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'build_command': ['cargo', 'build', '-p', 'romacs-vm-gui', '-p',
                                'romacs-gui-oracle', '-j', '2'],
              'scope': 'Package-scoped build. Workspace feature unification may enable legacy-core; not certified.',
              'records': records, 'nm': {'command': nm['command'], 'exit': 0,
                                        'romacs_core_symbols': 0},
              'later_assertion_mutants_killed': len(mutants)}
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'pass': True, 'binary_sha256': result['binary_sha256']}))


if __name__ == '__main__':
    main()
