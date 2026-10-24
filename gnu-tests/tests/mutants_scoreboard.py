"""Replay assertion-keyed CLI/fidelity mutants without changing the checkout.

Run with Python; --evidence-dir selects the external log/patch/JSON directory.
"""
import argparse
import ast
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


CASES = (
    ('remove-real-cli', 'test_real_cli_dispatch',
     'REAL-CLI: real harness must reach runner'),
    ('remove-fidelity-cli', 'test_fidelity_cli_dispatch',
     'FIDELITY-CLI: fidelity flag must reach runner'),
    ('bypass-defects-comparison', 'test_fidelity_checks_every_identity_and_verdict',
     'FIDELITY-DIFF: missing, extra and mismatched verdicts are harness defects'),
    ('bypass-zero-check', 'test_empty_fidelity_is_not_green',
     'FIDELITY-EMPTY: empty fidelity cannot be green'),
    ('drop-reference-inventory', 'test_runtime_inventory_must_match_reference',
     'REAL-INVENTORY: missing reference tests cannot disappear'),
    ('split-error-class', 'test_fidelity_uses_ert_failure_class',
     'FIDELITY-CLASS: errors are ERT failures, not harness defects'),
    ('accept-incomplete-process', 'test_scoreboard_process_failure_is_red',
     'REAL-PROCESS: incomplete process must make scoreboard red'),
    ('accept-vacuous-score', 'test_vacuous_pass_is_red_in_scoreboard_and_fidelity',
     'VACUOUS-SCORE: caught void-function is not parity'),
    ('accept-vacuous-fidelity', 'test_vacuous_pass_is_red_in_scoreboard_and_fidelity',
     'VACUOUS-FIDELITY: agreeing vacuous passes must be red'),
)


def mutate(source, name):
    if name == 'accept-vacuous-score':
        old = " and not result.get('vacuous_void_function')"
        assert source.count(old) == 1
        return source.replace(old, '')
    if name == 'accept-vacuous-fidelity':
        old = "\n            or real.get(name, {}).get('vacuous_void_function')\n            or mini.get(name, {}).get('vacuous_void_function')"
        assert source.count(old) == 1
        return source.replace(old, '')
    if name == 'accept-incomplete-process':
        old = 'if errors or len(reports) != len(files) or not rows:\n        return 4'
        assert source.count(old) == 1
        return source.replace(old, 'if errors or len(reports) != len(files) or not rows:\n        return 0')
    if name == 'drop-reference-inventory':
        old = 'sorted(reference.keys() | results.keys())'
        assert source.count(old) == 1
        return source.replace(old, 'sorted(results.keys())')
    if name == 'split-error-class':
        old = "return 'failed' if value == 'errored' else value"
        assert source.count(old) == 1
        return source.replace(old, 'return value')
    tree = ast.parse(source)
    if name.startswith('remove-'):
        flag = '--harness' if name == 'remove-real-cli' else '--harness-fidelity'
        targets = [node for node in ast.walk(tree)
                   if isinstance(node, ast.Expr) and isinstance(node.value, ast.Call)
                   and isinstance(node.value.func, ast.Attribute)
                   and node.value.func.attr == 'add_argument'
                   and node.value.args and isinstance(node.value.args[0], ast.Constant)
                   and node.value.args[0].value == flag]
        replacement = ''
    else:
        functions = [node for node in tree.body
                     if isinstance(node, ast.FunctionDef) and node.name == 'harness_defects']
        if len(functions) != 1:
            raise AssertionError('mutation requires exactly one harness_defects function')
        function = functions[0]
        kind = ast.Return if name == 'bypass-defects-comparison' else ast.If
        targets = [node for node in function.body if isinstance(node, kind)]
        replacement = '    return []\n' if kind is ast.Return else ''
    if len(targets) != 1:
        raise AssertionError(f'{name}: expected one mutation target, got {len(targets)}')
    target = targets[0]
    lines = source.splitlines(keepends=True)
    result = ''.join(lines[:target.lineno - 1]) + replacement + ''.join(lines[target.end_lineno:])
    if result == source:
        raise AssertionError(f'{name}: mutation did not apply')
    ast.parse(result)
    return result


def execute(root, test=None):
    command = [sys.executable, '-B', '-m', 'unittest', '-v',
               'test_scoreboard' + ('.Scoreboard.' + test if test else '')]
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1')
    env.pop('PYTHONPATH', None)
    proc = subprocess.run(command, cwd=root / 'gnu-tests' / 'tests', env=env,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, timeout=60)
    return command, proc


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence-dir', type=Path,
                        default=Path(tempfile.gettempdir()) / 'ert-scoreboard-evidence')
    args = parser.parse_args(argv)
    here = Path(__file__).resolve().parent
    checkout = here.parents[1]
    evidence = args.evidence_dir.resolve()
    if evidence == checkout or checkout in evidence.parents:
        parser.error('evidence must be outside the source checkout')
    evidence.mkdir(parents=True, exist_ok=True)
    source = (here.parent / 'run.py').read_text()
    tests = (here / 'test_scoreboard.py').read_text()
    report = {'source_sha256': hashlib.sha256(source.encode()).hexdigest(),
              'tests_sha256': hashlib.sha256(tests.encode()).hexdigest(), 'mutants': []}
    with tempfile.TemporaryDirectory(prefix='cli-mutants-', dir=evidence) as temporary:
        root = Path(temporary)
        isolated = root / 'gnu-tests'
        (isolated / 'tests').mkdir(parents=True)
        (isolated / 'tests' / 'test_scoreboard.py').write_text(tests)
        (isolated / 'run.py').write_text(source)
        (isolated / 'ert_report.py').write_text((here.parent / 'ert_report.py').read_text())
        (root / 'scripts').mkdir()
        (root / 'scripts/gnu_oracle.py').write_text((checkout / 'scripts/gnu_oracle.py').read_text())
        command, baseline = execute(root)
        (evidence / 'cli-mutants-baseline.log').write_text(baseline.stdout)
        report['baseline'] = {'command': command, 'returncode': baseline.returncode}
        baseline_ok = (baseline.returncode == 0 and
                       re.search(r'^Ran 8 tests? in ', baseline.stdout, re.MULTILINE) is not None)
        report['baseline']['passed'] = baseline_ok
        if baseline_ok:
            for name, test, message in CASES:
                mutated = mutate(source, name)
                (isolated / 'run.py').write_text(mutated)
                diff = ''.join(difflib.unified_diff(source.splitlines(keepends=True),
                                                   mutated.splitlines(keepends=True),
                                                   fromfile='original/run.py', tofile=name + '/run.py'))
                (evidence / f'cli-mutants-{name}.patch').write_text(diff)
                command, proc = execute(root, test)
                pattern = r'^AssertionError: (?:(?!\n-{5})[\s\S])*?' + re.escape(message) + r'$'
                matched = re.search(pattern, proc.stdout, re.MULTILINE)
                killed = (proc.returncode != 0 and matched is not None and
                          re.search(r'^Ran 1 test in ', proc.stdout, re.MULTILINE) is not None)
                (evidence / f'cli-mutants-{name}.log').write_text(proc.stdout)
                report['mutants'].append({'name': name, 'test': test, 'command': command,
                                          'returncode': proc.returncode, 'assertion_regex': pattern,
                                          'assertion': matched.group(0) if matched else None,
                                          'mutated_sha256': hashlib.sha256(mutated.encode()).hexdigest(),
                                          'killed': killed})
                print(f'{"KILLED" if killed else "SURVIVED"} {name}: {message}')
    passed = baseline_ok and len(report['mutants']) == len(CASES) and all(
        mutant['killed'] for mutant in report['mutants'])
    report['passed'] = passed
    (evidence / 'cli-mutants-summary.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f'baseline={"PASS" if baseline_ok else "FAIL"}; '
          f'killed={sum(mutant["killed"] for mutant in report["mutants"])}/{len(CASES)}')
    print(f'evidence: {evidence / "cli-mutants-summary.json"}')
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
