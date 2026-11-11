"""Replay assertion-keyed reporter mutants without editing the shared tree."""
import importlib.util
import io
import json
import pathlib
import sys
import re
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(tempfile.gettempdir()) / 'ert-reporter-mutants'
OUT.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(ROOT / 'gnu-tests'))
spec = importlib.util.spec_from_file_location('test_ert_report', ROOT / 'gnu-tests/tests/test_ert_report.py')
tests = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tests)
source = (ROOT / 'gnu-tests/ert_report.py').read_text()
mutants = [
    ('accept-absent', '    names = None', '    if not output:\n        return {}\n    names = None', 'Parser.test_absent_truncated_and_garbled_reports'),
    ('accept-garbled', "raise ReportError('garbled ERT protocol', output, evidence()) from error", 'return {}', 'Parser.test_absent_truncated_and_garbled_reports'),
    ('accept-zero', 'and bool(selected)', '', 'Parser.test_zero_registered_is_not_success'),
    ('ignore-death', "    names = None", "    returncode = 0 if returncode < 0 else returncode\n    names = None", 'Parser.test_death_even_with_complete_output'),
    ('ignore-status', "require(returncode == int(unexpected > 0),", 'require(True,', 'Parser.test_exit_status_must_match_unexpected_results'),
    ('missing-done', 'require(done and names is not None and native_summary is not None,', 'require(names is not None and native_summary is not None,', 'Parser.test_absent_truncated_and_garbled_reports'),
    ('ignore-native-total', 'require(total == len(names) == len(results),', 'require(True,', 'Parser.test_missing_row_and_summary_mismatch'),
    ('ignore-native-counts', 'require((expected, unexpected, skipped) == (actual_expected, actual_unexpected, actual_skipped),', 'require(True,', 'Parser.test_missing_row_and_summary_mismatch'),
    ('accept-unknown-outcome', "row.get('outcome') in ('passed', 'failed', 'skipped', 'quit')", 'True', 'Parser.test_invalid_row_types_and_unknown_outcome'),
    ('expected-failure-is-pass', "passed=sum(row['outcome'] == 'passed' for row in results.values())", "passed=sum(row['as_expected'] for row in results.values())", 'Parser.test_expected_failure_and_skip_are_raw_outcomes'),
    ('identity-count-only', 'name in names and name not in results', 'name not in results', 'Parser.test_registered_identity_mismatch'),
    ('ignore-start', "require(started and name == started[-1] and len(started) == len(results) + 1,", 'require(True,', 'Parser.test_starts_are_required_once_and_in_order'),
    ('missing-absence-audit', "vacuous = row.get('vacuous_void_function')", "vacuous = row.get('vacuous_void_function', [])", 'Parser.test_missing_absence_audit_is_not_empty_evidence'),
]
rows = []
for name, old, new, key in mutants:
    assert source.count(old) == 1, (name, source.count(old))
    mutated = source.replace(old, new)
    if name == 'ignore-death':
        mutated = mutated.replace('returncode in (0, 1)', '(returncode in (0, 1) or returncode < 0)')
    if name == 'identity-count-only':
        mutated = mutated.replace('set(results) == set(started) == set(names)', 'len(results) == len(started) == len(names)')
        mutated = mutated.replace('name in names and name not in started', 'name not in started')
        mutated = mutated.replace("require(name == names[len(started)],", "require(True,")
    if name == 'ignore-start':
        mutated = mutated.replace('set(results) == set(started) == set(names)', 'set(results) == set(names)')
    namespace = {'__name__': 'ert_report_mutant'}
    exec(compile(mutated, str(OUT / (name + '.py')), 'exec'), namespace)
    module = type(sys)('ert_report_mutant')
    module.__dict__.update(namespace)
    tests.ert_report = module
    suite = unittest.defaultTestLoader.loadTestsFromName(key, tests)
    stream = io.StringIO()
    result = unittest.TextTestRunner(stream=stream, verbosity=2).run(suite)
    message = ('invalid ERT report accepted' if name != 'expected-failure-is-pass' else 'ERT-RAW-OUTCOMES: expected failure must not become pass')
    killed = (bool(result.failures) and not result.errors
              and key.split('.')[-1] in stream.getvalue()
              and re.search(r'AssertionError: [\s\S]*' + re.escape(message), stream.getvalue()) is not None)
    (OUT / (name + '.txt')).write_text(stream.getvalue())
    rows.append(dict(mutant=name, assertion=key, killed_by_assertion=killed,
                     failures=len(result.failures), errors=len(result.errors)))
(OUT / 'mutants.json').write_text(json.dumps(rows, indent=2) + '\n')
print(json.dumps(rows, indent=2))
assert all(row['killed_by_assertion'] for row in rows)
