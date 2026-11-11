#!/usr/bin/env python3
"""Prove each of the runner's own tests can fail: apply one mutation that
removes what the test protects, run the test, require it to FAIL, restore.

    ROMACS_GNU_ERT=<adapter> python3 gnu-tests/tests/mutants.py [NAME...]

Exit 0 only if every mutant was killed (its targeted test failed) AND the
unmutated suite passes afterwards.  Files are restored byte-for-byte in a
finally block even when a run is interrupted.
"""
from __future__ import annotations

import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
GT = HERE.parent
RUN = GT / 'run.py'
HARNESS = GT / 'harness' / 'romacs-mini-ert.el'

# name: (file, old, new, tests that must fail)
MUTANTS = {
    'classify-always-passes': (
        RUN, "    if row['verdict'] == 'passed':\n        if exclusion:",
        "    row['verdict'] = 'passed'\n    if row['verdict'] == 'passed':\n        if exclusion:",
        ['Classification.test_assertion_failure_is_failed_not_passed',
         'Classification.test_never_run_is_not_yet_runnable']),
    'no-harness-fidelity-check': (
        RUN, "    elif fidelity is None or fidelity['outcome'] not in {g['outcome'] for g in regimes}:",
        "    elif False:",
        ['Classification.test_harness_infidelity_blocks_a_romacs_pass']),
    'romacs-skip-counts-as-pass': (
        RUN, "        if any(r == x['outcome'] and x.get('as_expected')",
        "        if r == 'skipped' or any(r == x['outcome'] and x.get('as_expected')",
        ['Classification.test_romacs_skip_of_a_gnu_pass_is_not_passed']),
    'never-run-inherits-gnu-outcome': (
        RUN, "        r = romacs.get(name)\n",
        "        r = romacs.get(name) or (g if run is not None else None)\n",
        ['EndToEnd.test_an_unreadable_file_reports_nothing_as_passed',
         'EndToEnd.test_an_adapter_that_runs_nothing_reports_nothing_as_passed']),
    'summary-counts-not-runnable-as-passed': (
        RUN, "            'passed_on_romacs': sum(1 for r in rows if r['verdict'] == 'passed'),\n            'runnable",
        "            'passed_on_romacs': sum(1 for r in rows if r['verdict'] in ('passed', 'not-yet-runnable')),\n            'runnable",
        ['Classification.test_summary_never_counts_not_runnable_as_passed']),
    'ledger-accepts-unknown-divergence': (
        RUN, "        if div is None:\n            errors.append(",
        "        if div is None:\n            pass\n        if False:\n            errors.append(",
        ['Ledger.test_an_unknown_divergence_is_refused']),
    'ledger-skips-symbol-use-check': (
        RUN, "            elif not re.search(r'(?<![-\\w])' + re.escape(row['symbol'])",
        "            elif False and not re.search(r'(?<![-\\w])' + re.escape(row['symbol'])",
        ['Ledger.test_a_symbol_the_test_never_uses_is_refused']),
    'ledger-accepts-missing-why': (
        RUN, "            if not isinstance(value, str) or not value.strip():",
        "            if isinstance(value, str) and not value.strip():",
        ['Ledger.test_a_missing_justification_is_refused']),
    'ledger-keeps-stale-exclusion': (
        RUN, "        if exclusion:\n            raise Refusal(2, f'{exclusion[\"file\"]}::{name} PASSES",
        "        if False:\n            raise Refusal(2, f'{exclusion[\"file\"]}::{name} PASSES",
        ['Ledger.test_an_excluded_test_that_passes_is_refused']),
    'suspect-ledger-entries-trusted': (
        RUN, "        if row['divergence'] in suspects:\n",
        "        if False:\n",
        ['Ledger.test_a_row_citing_a_suspect_entry_is_unverified_debt']),
    'gnu-source-hash-not-checked': (
        RUN, "            elif actual != row['gnu_source_sha256']:",
        "            elif False:",
        ['Ledger.test_a_citation_that_is_not_the_verified_code_is_refused']),
    'unverified-exclusion-counts-as-divergence': (
        RUN, "        row['category'] = exclusion['category'] if exclusion.get('verified') else 'excluded-unverified-row'",
        "        row['category'] = exclusion['category']",
        ['Ledger.test_a_row_citing_a_suspect_entry_is_unverified_debt']),
    'single-gnu-regime-only': (
        RUN, "    regimes = [g for g in (gnu, gnu_compiled) if g and g['outcome'] != 'not-recorded']",
        "    regimes = [g for g in (gnu,) if g and g['outcome'] != 'not-recorded']",
        ['Classification.test_matching_either_gnu_regime_is_parity']),
    'pin-not-checked': (
        RUN, "    if actual != p['test_tree']:\n",
        "    if False:\n",
        ['Pin.test_an_edited_import_is_refused', 'Pin.test_a_stray_file_in_the_import_is_refused']),
    'timeout-recorded-as-pass': (
        RUN, "            outcome = 'panic' if panic and panic.get('test') == current else 'timeout'",
        "            outcome = 'panic' if panic and panic.get('test') == current else 'passed'",
        ['EndToEnd.test_a_hung_test_times_out_and_later_tests_still_run']),
    'harness-should-never-fails': (
        HARNESS, "  (if value value (romacs-gnu-tests--fail (list 'should form :value value))))",
        "  (or value t))",
        ['EndToEnd.test_each_verdict_is_reported']),
    'harness-ert-info-drops-its-forms': (
        HARNESS, "    (cons 'progn (cons prefix-form (cons message-form body)))))",
        "    (cons 'progn body)))",
        ['EndToEnd.test_each_verdict_is_reported']),
    'harness-should-not-never-fails': (
        HARNESS, "  (if value (romacs-gnu-tests--fail (list 'should-not form :value value)) nil))",
        "  nil)",
        ['EndToEnd.test_a_genuine_failure_is_reported_as_failed']),
    'harness-should-error-ignores-type': (
        HARNESS, "    (if type\n        (let ((types",
        "    (if nil\n        (let ((types",
        ['EndToEnd.test_each_verdict_is_reported']),
}


def run_tests(tests: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, str(HERE / 'test_run.py'), *tests],
                          capture_output=True, text=True, timeout=900)


def main(names: list[str]) -> int:
    names = names or list(MUTANTS)
    survivors = []
    for name in names:
        path, old, new, tests = MUTANTS[name]
        original = path.read_bytes()
        text = original.decode()
        if text.count(old) != 1:
            print(f'{name}: STALE (mutation site found {text.count(old)} times)')
            survivors.append(name)
            continue
        try:
            path.write_text(text.replace(old, new))
            p = run_tests(tests)
        finally:
            path.write_bytes(original)
        killed = p.returncode != 0
        tail = [l for l in p.stderr.splitlines() if l.startswith(('FAIL:', 'ERROR:', 'AssertionError', 'Ran ', 'FAILED', 'OK'))]
        print(f'{name}: {"KILLED" if killed else "SURVIVED"} (exit {p.returncode}) {" | ".join(tail)[:600]}')
        if not killed:
            survivors.append(name)
    clean = run_tests([])
    print(f'unmutated suite: exit {clean.returncode} {clean.stderr.strip().splitlines()[-1] if clean.stderr.strip() else ""}')
    if survivors:
        print('SURVIVING MUTANTS: ' + ', '.join(survivors))
    return 0 if not survivors and clean.returncode == 0 else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
