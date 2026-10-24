"""Kill timeout attribution mutants with runtime assertions, without editing inputs."""
import contextlib
import io
import unittest
from unittest.mock import patch
import primh
import test_primh

source = primh.Path(primh.__file__).read_text()
mutants = {
    'skip-isolated-retry': (
        'retry, retry_timed_out = run(todo[:1], 10)',
        'retry, retry_timed_out = [], True',
        'test_individual_hang_excludes_only_confirmed_case'),
    'retry-whole-tail': (
        'retry, retry_timed_out = run(todo[:1], 10)',
        'retry, retry_timed_out = run(todo, 10)',
        'test_individual_hang_excludes_only_confirmed_case'),
    'drop-streamed-regime': (
        'got += done\n                todo =',
        "got += [dict(g, interp=g['expected']) for g in done]\n                todo =",
        'test_aggregate_timeout_retries_terminating_case_alone'),
    'drop-retry-regime': (
        'got += retry',
        "got += [dict(g, interp=g['expected']) for g in retry]",
        'test_isolated_timeout_after_complete_row_is_not_excluded'),
    'accept-wrong-header': (
        "if h.get('subrs') != [[spec['name'], 'subr', *spec['arity']]] or (head is not None and h != head):",
        'if False:',
        'test_isolated_retry_header_is_validated'),
    'hide-vm-crash': (
        "    if actual is None:\n        return 'VM-CRASH'\n",
        '',
        'test_vm_crash_is_not_valid_timeout_exclusion'),
}
for name, (old, new, test) in mutants.items():
    assert source.count(old) == 1, name
    namespace = dict(primh.__dict__, __name__='primh_mutant')
    exec(compile(source.replace(old, new), primh.__file__, 'exec'), namespace)
    suite = unittest.TestSuite([test_primh.RecordingTests(test)])
    log = io.StringIO()
    with patch.object(primh, 'gnu_rows', namespace['gnu_rows']), patch.object(primh, 'verdict', namespace['verdict']):
        with contextlib.redirect_stdout(log):
            result = unittest.TextTestRunner(stream=log, verbosity=2).run(suite)
    print(f'=== MUTANT {name} ===\n{log.getvalue()}')
    assert result.failures and not result.errors, f'{name}: not killed by an assertion'
print('All six timeout implementation mutants killed by runtime assertions.')
