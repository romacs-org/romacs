"""Integrity-checked GNU ERT batch reports (not the reduced mini harness).

Counts describe raw outcomes; :expected-result is retained independently in
``as_expected``.  A completed suite with unexpected results is a valid report,
not an execution-integrity error.  ReportError retains all captured diagnostics.
"""
from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess

PREFIX = 'ERT-SCOREBOARD '
SUMMARY = re.compile(
    r'^Ran (\d+) tests, (\d+) results as expected, (\d+) unexpected'
    r'(?:, (\d+) skipped)? \([^\n]+, [0-9.]+ sec\)$')


class ReportError(ValueError):
    def __init__(self, message: str, output: str = ''):
        super().__init__(message)
        self.output = output


def parse_report(output: str, returncode: int) -> dict:
    """Require inventory, one result per identity, native summary and completion.

    The protocol is emitted by an observer of GNU's own batch listener, never a
    replacement evaluator.  Nonprotocol diagnostic text is retained, not parsed
    as test success.  Skips are excluded from GNU's expected/unexpected totals.
    """
    def require(condition, message):
        if not condition:
            raise ReportError(message, output)

    require(type(returncode) is int and returncode in (0, 1), 'abnormal ERT process exit')
    names = None
    results = {}
    native_summary = None
    done = False
    for line in output.splitlines():
        if line.startswith(PREFIX):
            try:
                row = json.loads(line[len(PREFIX):])
            except (ValueError, TypeError) as error:
                raise ReportError('garbled ERT protocol', output) from error
            require(isinstance(row, dict), 'protocol row must be an object')
            kind = row.get('event')
            require(not done, 'protocol after completion')
            if kind == 'registered':
                require(names is None and not results and native_summary is None,
                        'duplicate or late registration inventory')
                names = row.get('names')
                require(isinstance(names, list) and bool(names)
                        and all(isinstance(name, str) and name for name in names),
                        'empty or invalid registration inventory')
                require(len(set(names)) == len(names), 'duplicate registered identity')
            elif kind == 'result':
                require(names is not None and native_summary is None, 'result outside run')
                name = row.get('test')
                require(isinstance(name, str) and name in names and name not in results,
                        'unexpected or duplicate result identity')
                require(row.get('outcome') in ('passed', 'failed', 'skipped', 'quit')
                        and type(row.get('as_expected')) is bool, 'invalid ERT result')
                results[name] = dict(outcome=row['outcome'], as_expected=row['as_expected'])
                vacuous = row.get('vacuous_void_function', [])
                require(isinstance(vacuous, list) and all(isinstance(x, str) and x for x in vacuous),
                        'invalid vacuous-pass evidence')
                if vacuous:
                    results[name]['vacuous_void_function'] = vacuous
            elif kind == 'done':
                require(names is not None and native_summary is not None, 'premature completion')
                done = True
            else:
                raise ReportError('unknown ERT protocol event', output)
        elif line.startswith(('Ran ', 'Aborted: Ran ')):
            match = SUMMARY.fullmatch(line)
            require(match is not None and native_summary is None and not done,
                    'missing, duplicate or invalid native ERT summary')
            require(names is not None, 'summary before inventory')
            native_summary = tuple(int(value or 0) for value in match.groups())
    require(done and names is not None and native_summary is not None, 'incomplete ERT report')
    require(set(results) == set(names), 'registered/result identities differ')
    total, expected, unexpected, skipped = native_summary
    require(total == len(names) == len(results), 'native summary total differs from inventory')
    actual_skipped = sum(row['outcome'] == 'skipped' for row in results.values())
    actual_expected = sum(row['as_expected'] and row['outcome'] != 'skipped'
                          for row in results.values())
    actual_unexpected = len(results) - actual_expected - actual_skipped
    require((expected, unexpected, skipped) == (actual_expected, actual_unexpected, actual_skipped),
            'native summary differs from result rows')
    require(returncode == int(unexpected > 0), 'exit status differs from native summary')
    summary = dict(passed=sum(row['outcome'] == 'passed' for row in results.values()),
                   failed=sum(row['outcome'] in ('failed', 'quit') for row in results.values()),
                   skipped=actual_skipped, total=total)
    return dict(results=results, summary=summary, registered=len(names), complete=True)


def run_native(binary, rel_path: pathlib.Path, mini_harness: pathlib.Path | None,
               ert_path: pathlib.Path, load_paths: list[str], home: pathlib.Path,
               timeout: float, environment: dict | None = None) -> dict:
    """Load pinned ERT source, load the test file, then run real batch ERT.

    ``rel_path`` resolves relative to the caller's cwd.  Mini fidelity remains a
    separate runner; passing a mini harness here is an explicit error.
    """
    if mini_harness is not None:
        raise ReportError('native ERT reporting does not execute the mini harness')
    ert_path = pathlib.Path(ert_path).resolve()
    target = pathlib.Path(rel_path).resolve()
    if not ert_path.is_file() or not target.is_file():
        raise ReportError('missing pinned ERT source or test file')
    if timeout <= 0:
        raise ReportError('native ERT timeout must be positive')
    # JSON strings are valid Emacs Lisp string literals for ordinary filesystem
    # paths; ensure_ascii=False avoids JSON-only Unicode escape syntax.
    quote = lambda value: json.dumps(str(value), ensure_ascii=False)
    reporter = (pathlib.Path(__file__).parent / 'harness/romacs-real-ert-reporter.el').read_text()
    expression = (f'(progn (load {quote(ert_path)} nil t t) '
                  f'(load {quote(target)} nil t t) (kill-emacs (progn {reporter})))')
    command = [str(binary), '-Q', '--batch']
    for path in load_paths:
        command.extend(['-L', str(path)])
    command.extend(['--eval', expression])
    env = dict(os.environ if environment is None else environment)
    env.pop('EMACSLOADPATH', None)
    env['LANG'] = 'C'
    env['HOME'] = str(home)
    # Avoid test report side effects inherited from the caller.
    env.pop('EMACS_TEST_JUNIT_REPORT', None)
    try:
        process = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                 encoding='utf-8', errors='replace', timeout=timeout, env=env,
                                 cwd=target.parent)
    except subprocess.TimeoutExpired as error:
        output = error.output or b''
        if isinstance(output, bytes):
            output = output.decode('utf-8', errors='replace')
        raise ReportError('native ERT timed out', output) from error
    except OSError as error:
        raise ReportError(f'cannot execute native ERT: {error}') from error
    result = parse_report(process.stdout, process.returncode)
    result.update(output=process.stdout, returncode=process.returncode, status='complete')
    return result
