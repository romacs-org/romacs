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
    def __init__(self, message: str, output: str = '', evidence: dict | None = None):
        super().__init__(message)
        self.output = output
        self.evidence = evidence or dict(selected=[], started=[], terminal=[], results={},
                                        execution=dict(selected=0, started=0, terminal=0),
                                        complete=False)


def parse_report(output: str, returncode: int) -> dict:
    """Require selection, starts, results, native summary and completion.

    The protocol is emitted by an observer of GNU's own batch listener, never a
    replacement evaluator.  Nonprotocol diagnostic text is retained, not parsed
    as test success.  Skips are excluded from GNU's expected/unexpected totals.
    """
    names = None
    started = []
    results = {}

    def evidence():
        return dict(selected=list(names or []), started=list(started), terminal=list(results),
                    results=dict(results), execution=dict(selected=len(names or []),
                                                         started=len(started), terminal=len(results)),
                    complete=False)

    def require(condition, message):
        if not condition:
            raise ReportError(message, output, evidence())

    native_summary = None
    done = False
    for line in output.split('\n'):
        if line.startswith(PREFIX):
            try:
                row = json.loads(line[len(PREFIX):])
            except (ValueError, TypeError) as error:
                raise ReportError('garbled ERT protocol', output, evidence()) from error
            require(isinstance(row, dict), 'protocol row must be an object')
            kind = row.get('event')
            require(not done, 'protocol after completion')
            if kind == 'selected':
                require(names is None and not started and not results and native_summary is None,
                        'duplicate or late selection inventory')
                selected = row.get('names')
                require(isinstance(selected, list) and bool(selected)
                        and all(isinstance(name, str) and name for name in selected),
                        'empty or invalid selection inventory')
                require(len(set(selected)) == len(selected), 'duplicate selected identity')
                names = selected
            elif kind == 'started':
                require(names is not None and native_summary is None, 'start outside run')
                name = row.get('test')
                require(isinstance(name, str) and name in names and name not in started,
                        'unexpected or duplicate start identity')
                require(name == names[len(started)], 'start differs from selected order')
                require(len(started) == len(results), 'start before previous terminal result')
                started.append(name)
            elif kind == 'result':
                require(names is not None and native_summary is None, 'result outside run')
                name = row.get('test')
                require(isinstance(name, str) and name in names and name not in results,
                        'unexpected or duplicate result identity')
                require(started and name == started[-1] and len(started) == len(results) + 1,
                        'result without matching start')
                require(row.get('outcome') in ('passed', 'failed', 'skipped', 'quit')
                        and type(row.get('as_expected')) is bool, 'invalid ERT result')
                vacuous = row.get('vacuous_void_function')
                require(isinstance(vacuous, list) and all(isinstance(x, str) and x for x in vacuous),
                        'invalid vacuous-pass evidence')
                results[name] = dict(outcome=row['outcome'], as_expected=row['as_expected'])
                if vacuous:
                    results[name]['vacuous_void_function'] = vacuous
            elif kind == 'done':
                require(names is not None and native_summary is not None, 'premature completion')
                done = True
            else:
                raise ReportError('unknown ERT protocol event', output, evidence())
        elif line.startswith(('Ran ', 'Aborted: Ran ')):
            match = SUMMARY.fullmatch(line)
            require(match is not None and native_summary is None and not done,
                    'missing, duplicate or invalid native ERT summary')
            require(names is not None, 'summary before inventory')
            native_summary = tuple(int(value or 0) for value in match.groups())
    require(type(returncode) is int and returncode in (0, 1), 'abnormal ERT process exit')
    require(done and names is not None and native_summary is not None, 'incomplete ERT report')
    require(set(results) == set(started) == set(names), 'selected/started/result identities differ')
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
    result = evidence()
    result.update(summary=summary, registered=len(names), complete=True)
    return result


def run_native(binary, rel_path: pathlib.Path, mini_harness: pathlib.Path | None,
               ert_path: pathlib.Path, load_paths: list[str], home: pathlib.Path,
               timeout: float, environment: dict | None = None, selector: str = 't') -> dict:
    """Load pinned ERT source, load the test file, then run real batch ERT.

    ``rel_path`` resolves relative to the caller's cwd.  Mini fidelity remains a
    separate runner; passing a mini harness here is an explicit error.
    SELECTOR is a Lisp expression evaluated once before the batch run.
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
                  f'(load {quote(target)} nil t t) '
                  f'(setq ert-scoreboard-selector {selector}) '
                  f'(kill-emacs (progn {reporter})))')
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
        try:
            parse_report(output, -1)
        except ReportError as partial:
            raise ReportError('native ERT timed out', output, partial.evidence) from error
    except OSError as error:
        raise ReportError(f'cannot execute native ERT: {error}') from error
    result = parse_report(process.stdout, process.returncode)
    result.update(output=process.stdout, returncode=process.returncode, status='complete')
    return result
