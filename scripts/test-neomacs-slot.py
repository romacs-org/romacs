#!/usr/bin/env python3
"""Run through proc-sandbox: only our recorded test children are inspected."""
import importlib.util
from importlib.machinery import SourceFileLoader
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from typing import Any

sys.dont_write_bytecode = True
SOURCE = Path(__file__).with_name('neomacs-slot-runner.py')
OLD_SOURCE = Path(os.environ.get('NEOMACS_SLOT_OLD_RUNNER', str(SOURCE)))
REPORT_SOURCE = Path(os.environ.get('NEOMACS_SLOT_REPORT_SOURCE', str(Path(__file__).with_name('cockpit-velocity'))))
CANDIDATE = SOURCE.with_suffix('.py.new')
if CANDIDATE.exists():
    SOURCE = CANDIDATE
SOURCE = Path(os.environ.get('NEOMACS_SLOT_TEST_SOURCE', str(SOURCE)))
# S1 keeps all scope and cache effects in scratch; no systemd job is launched.
def load_runner(source=SOURCE) -> Any:
    spec = importlib.util.spec_from_file_location('runner', source, loader=SourceFileLoader('runner', str(source)))
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def wait_for_sigterm(ready):
    """Test fixture signal boundary; notify only when ready to wait."""
    import signal
    # Block before notification: pause() loses a signal delivered between its
    # GIL release and the pause syscall. sigwait consumes even an early signal.
    previous = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM})
    try:
        ready()
        received = signal.sigwait({signal.SIGTERM})
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)
    signal.raise_signal(received)  # Exercise the runner's actual handler.


if len(sys.argv) > 1 and sys.argv[1] == 'ledger-child':
    source, directory, lane = sys.argv[2:]
    child_root = Path(directory)
    module = load_runner(Path(source))
    module.STATE, module.LEGACY = child_root / 'queue', child_root
    module.Path.home = classmethod(lambda cls: child_root / 'home')
    module.shutil.which = lambda _: '/bin/true'
    def fake_systemctl(*argv):
        marker = child_root / (argv[1] + '.alive')
        if argv[0] == 'stop':
            assert list(module.STATE.glob('*.json'))
            marker.unlink(missing_ok=True)
        return subprocess.CompletedProcess(argv, 0,
            'ActiveState=active\n' if marker.exists() else 'ActiveState=inactive\n', '')
    module.systemctl = fake_systemctl
    def scoped_payload(argv, **kwargs):
        unit = next(a.split('=', 1)[1] for a in argv if a.startswith('--unit='))
        marker = child_root / (unit + '.scope.alive')
        marker.touch()
        with module.transaction():
            assert sum(t['active'] for _, t in module.records()) <= 2
        def ready():
            print(json.dumps({'lane': lane, 'unit': unit}), flush=True)
        if lane == 'signal':
            wait_for_sigterm(ready)
        else:
            ready()
        assert sys.stdin.readline().strip() == 'release'
        # Neither version may remove a ticket while its scope is alive.
        assert any(json.loads(p.read_text())['unit'] == unit for p in module.STATE.glob('*.json'))
        marker.unlink()
        return 0
    module.subprocess.call = scoped_payload
    sys.argv = [source, lane, '--slots', '2', '--mem', '1M', '--', 'true']
    sys.exit(module.main())


if len(sys.argv) > 1 and sys.argv[1] == '--s1-tests':
    import contextlib
    import select
    import signal
    import unittest
    from unittest.mock import patch

    class LedgerTests(unittest.TestCase):
        def exercise(self, outcome, blocked=False, queued=False, poll_step=2,
                     queue_unlocked=False, closed_stderr=None, admissible_head=False):
            expected = 128 - outcome if outcome < 0 else outcome
            module = load_runner()
            with tempfile.TemporaryDirectory(prefix='velocity-S1-', dir=os.environ['TMPDIR']) as directory:
                root = Path(directory)
                state = root / 'queue'
                state.mkdir()
                if blocked:
                    (state / 'jobs.jsonl').mkdir()  # Fails even when run as root.
                calls = []
                def ctl(*argv):
                    if argv[0] == 'stop':
                        self.assertTrue(list(state.glob('*.json')), 'ticket removed before scope teardown')
                        calls.append('stop')
                    return subprocess.CompletedProcess(argv, 0, 'ActiveState=inactive\n', '')
                def payload(*a, **kw):
                    if outcome == 130:
                        raise KeyboardInterrupt
                    return outcome
                real_records = module.records
                def records():
                    own = real_records()
                    head = dict(own[0][1], pid=-1, group='head', created=0,
                                high=1024**2 if admissible_head else 10**20)
                    return [(None, head), *own]
                clock = [0]
                def queued_interrupt(_):
                    clock[0] += poll_step
                    if clock[0] == 3 * poll_step:
                        raise KeyboardInterrupt
                old_signals = {s: signal.getsignal(s) for s in (signal.SIGINT, signal.SIGTERM)}
                try:
                    with contextlib.ExitStack() as stack:
                        stack.enter_context(patch.object(module, 'STATE', state))
                        stack.enter_context(patch.object(module, 'LEGACY', root, create=True))
                        stack.enter_context(patch.object(Path, 'home', return_value=root / 'home'))
                        stack.enter_context(patch.object(module.shutil, 'which', return_value='/bin/true'))
                        stack.enter_context(patch.object(module.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)))
                        stack.enter_context(patch.object(module.subprocess, 'call', side_effect=payload))
                        stack.enter_context(patch.object(module, 'systemctl', side_effect=ctl))
                        if hasattr(module, 'append_event'):
                            append = module.append_event
                            def checked_append(event):
                                if 'finished' in event:
                                    self.assertFalse(list(state.glob('*.json')))
                                    self.assertFalse(module.scope_alive(event['unit']))
                                if queue_unlocked:
                                    import fcntl
                                    old = load_runner(OLD_SOURCE)
                                    old.STATE = state
                                    with (state / 'queue.lock').open('a') as probe:
                                        fcntl.flock(probe, fcntl.LOCK_EX | fcntl.LOCK_NB)
                                    with old.transaction():
                                        old.records()
                                if closed_stderr and 'finished' in event:
                                    if closed_stderr == 'pipe':
                                        reader, writer = os.pipe()
                                        os.close(reader)
                                        stream = os.fdopen(writer, 'w', buffering=1)
                                    else:
                                        stream = open(root / 'stderr', 'w')
                                        stream.close()
                                    try:
                                        with patch.object(sys, 'stderr', stream):
                                            append(event)
                                    finally:
                                        with contextlib.suppress(OSError):
                                            stream.close()
                                else:
                                    append(event)
                            stack.enter_context(patch.object(module, 'append_event', side_effect=checked_append))
                        stack.enter_context(patch.object(sys, 'argv', ['runner', 'velocity-S1', '--mem', '1M', '--', 'true']))
                        if queued:
                            stack.enter_context(patch.object(module, 'records', side_effect=records))
                            stack.enter_context(patch.object(module.time, 'sleep', side_effect=queued_interrupt))
                            stack.enter_context(patch.object(module.time, 'monotonic', side_effect=lambda: clock[0]))
                        try:
                            rc = module.main()
                        except (OSError, ValueError) as error:
                            rc = type(error).__name__
                    self.assertEqual(rc, expected, 'ledger failure changed the caller rc')
                    self.assertFalse(list(state.glob('*.json')), 'ledger failure leaked ticket')
                    if not queued:
                        self.assertEqual(calls, ['stop'])
                    if not blocked:
                        rows = [json.loads(line) for line in (state / 'jobs.jsonl').read_text().splitlines()]
                        finished = [r for r in rows if 'finished' in r]
                        self.assertEqual(len(finished), 1)
                        row = finished[0]
                        self.assertEqual(row['rc'], expected)
                        self.assertEqual(row['lane'], 'velocity-S1')
                        self.assertEqual(row['group'], 'velocity')
                        self.assertEqual(row['mode'], ['true'])
                        self.assertIsInstance(row['priority'], int)
                        self.assertEqual(row['seq'], 1)
                        self.assertLessEqual(row['created'], row['finished'])
                        self.assertIn('unit', row)
                        self.assertFalse(any(r.get('event') == 'hol' or 'hol_spans' in r for r in rows))
                        if queued:
                            self.assertIsNone(row['admitted'])
                        else:
                            self.assertLessEqual(row['created'], row['admitted'])
                            self.assertLessEqual(row['admitted'], row['finished'])
                finally:
                    for s, handler in old_signals.items():
                        signal.signal(s, handler)

        def test_m1_ledger_outside_queue_transaction(self):
            self.exercise(130, queued=True, queue_unlocked=True)
            self.exercise(0, queue_unlocked=True)

        def test_m1_held_ledger_does_not_stall_old_or_new(self):
            import fcntl
            with tempfile.TemporaryDirectory(prefix='velocity-lock-', dir=os.environ['TMPDIR']) as directory:
                root = Path(directory)
                state = root / 'queue'
                state.mkdir()
                with (state / 'jobs.jsonl').open('a') as ledger:
                    fcntl.flock(ledger, fcntl.LOCK_EX)
                    process = subprocess.Popen([sys.executable, __file__, 'ledger-child', str(SOURCE), directory, 'lock'],
                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    try:
                        assert process.stdin is not None and process.stdout is not None and process.stderr is not None
                        self.assertTrue(select.select([process.stdout], [], [], 5)[0])
                        self.assertIn('unit', json.loads(process.stdout.readline()))
                        old = load_runner(OLD_SOURCE)
                        old.STATE = state
                        with old.transaction():
                            self.assertEqual(len(old.records()), 1)
                        process.stdin.write('release\n')
                        process.stdin.flush()
                        self.assertEqual(process.wait(timeout=2), 0)
                        with old.transaction():
                            self.assertEqual(old.records(), [])
                    finally:
                        if process.poll() is None:
                            process.kill()
                            process.wait()
                        for stream in (process.stdin, process.stdout, process.stderr):
                            if stream is not None:
                                stream.close()

        def test_m1_single_bounded_nonblocking_write(self):
            from types import SimpleNamespace
            module = load_runner()
            module.args = SimpleNamespace(lane='append-test')
            with tempfile.TemporaryDirectory(prefix='velocity-append-', dir=os.environ['TMPDIR']) as directory:
                module.STATE = Path(directory)
                with patch.object(module.os, 'write', wraps=os.write) as write:
                    module.append_event(dict(unit='small'))
                    self.assertEqual(write.call_count, 1)
                    self.assertLessEqual(len(write.call_args.args[1]), select.PIPE_BUF)
                    write.reset_mock()
                    module.append_event(dict(unit='x' * (select.PIPE_BUF + 1)))
                    write.assert_not_called()
                fifo = module.STATE / 'jobs.jsonl'
                fifo.unlink()
                os.mkfifo(fifo)
                # No reader: O_NONBLOCK must return ENXIO, not wait for an open.
                with patch.object(module.os, 'open', wraps=os.open) as opened:
                    module.append_event(dict(unit='fifo'))
                    self.assertTrue(opened.call_args.args[1] & os.O_NONBLOCK)
                    self.assertTrue(opened.call_args.args[1] & os.O_APPEND)

        def test_m2_closed_stderr_ledger_failure_preserves_rc(self):
            for stream in ('pipe', 'closed'):
                for rc in (0, 1, 130):
                    with self.subTest(stream=stream, rc=rc):
                        self.exercise(rc, blocked=True, closed_stderr=stream)

        def test_m4_hol_not_emitted_for_admissible_head(self):
            self.exercise(130, queued=True, admissible_head=True)

        def test_sigterm_pending_before_wait(self):
            module = load_runner()
            before = signal.getsignal(signal.SIGTERM)
            def ready():
                mask = signal.pthread_sigmask(signal.SIG_BLOCK, set())
                self.assertIn(signal.SIGTERM, mask, 'notification precedes atomic signal wait')
                os.kill(os.getpid(), signal.SIGTERM)
                self.assertIn(signal.SIGTERM, signal.sigpending())
            try:
                signal.signal(signal.SIGTERM, module.interrupted)
                with self.assertRaises(KeyboardInterrupt):
                    wait_for_sigterm(ready)
                self.assertNotIn(signal.SIGTERM, signal.pthread_sigmask(signal.SIG_BLOCK, set()))
            finally:
                signal.signal(signal.SIGTERM, before)

        def test_unwritable_ledger_preserves_rc_and_cleanup(self):
            for rc in (0, 1, 130):
                with self.subTest(rc=rc):
                    self.exercise(rc, blocked=True)

        def test_completion_and_interrupt_ledger(self):
            for rc in (0, 1, 130, -15):
                with self.subTest(rc=rc):
                    self.exercise(rc)

        def test_queued_interrupt_completion_only(self):
            self.exercise(130, queued=True)

        def test_queue_notice_rate_limit(self):
            self.exercise(130, queued=True, poll_step=150)
            self.exercise(130, blocked=True, queued=True)

        def test_sigterm_interrupt(self):
            with tempfile.TemporaryDirectory(prefix='velocity-signal-', dir=os.environ['TMPDIR']) as directory:
                process = subprocess.Popen([sys.executable, __file__, 'ledger-child', str(SOURCE), directory, 'signal'],
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                try:
                    assert process.stdout is not None
                    readable, _, _ = select.select([process.stdout], [], [], 10)
                    self.assertTrue(readable)
                    unit = json.loads(process.stdout.readline())['unit']
                    process.send_signal(signal.SIGTERM)
                    self.assertEqual(process.wait(timeout=10), 130)
                    root = Path(directory)
                    self.assertFalse((root / (unit + '.scope.alive')).exists())
                    self.assertFalse(list((root / 'queue').glob('*.json')))
                    rows = [json.loads(line) for line in (root / 'queue/jobs.jsonl').read_text().splitlines()]
                    self.assertEqual(rows[-1]['rc'], 130)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()
                    for stream in (process.stdin, process.stdout, process.stderr):
                        if stream is not None:
                            stream.close()

        def test_mixed_old_new_children(self):
            with tempfile.TemporaryDirectory(prefix='velocity-mixed-', dir=os.environ['TMPDIR']) as directory:
                children = [subprocess.Popen([sys.executable, __file__, 'ledger-child', str(source), directory, f'mix{i}'],
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    for i, source in enumerate([OLD_SOURCE, SOURCE] * 3)]
                pending = {}
                for process in children:
                    assert process.stdout is not None and process.stdin is not None and process.stderr is not None
                    pending[process.stdout] = process
                count = 0
                try:
                    while pending:
                        readable, _, _ = select.select(list(pending), [], [], 20)
                        self.assertTrue(readable, 'mixed scheduler admission stalled')
                        for stream in readable:
                            process = pending.pop(stream)
                            assert process.stdin is not None and process.stderr is not None
                            self.assertTrue(json.loads(stream.readline())['unit'].startswith('neomacs-slot-'))
                            tickets = [json.loads(p.read_text()) for p in (Path(directory) / 'queue').glob('*.json')]
                            self.assertLessEqual(sum(t['active'] for t in tickets), 2)
                            process.stdin.write('release\n')
                            process.stdin.flush()
                            rc = process.wait(timeout=10)
                            self.assertEqual(rc, 0, process.stderr.read())
                            count += 1
                    self.assertEqual(count, 6)
                    self.assertFalse(list((Path(directory) / 'queue').glob('*.json')))
                    rows = [json.loads(line) for line in (Path(directory) / 'queue/jobs.jsonl').read_text().splitlines()]
                    expected = sum(hasattr(load_runner(source), 'append_event') for source in [OLD_SOURCE, SOURCE] * 3)
                    self.assertEqual(sum('finished' in r for r in rows), expected)
                finally:
                    for process in children:
                        if process.poll() is None:
                            process.kill()
                            process.wait()
                        for stream in (process.stdin, process.stdout, process.stderr):
                            if stream is not None:
                                stream.close()

    @unittest.skipUnless(REPORT_SOURCE.exists(),
                         'cockpit report is not part of the repo-owned slot runner')
    class ReportTests(unittest.TestCase):
        def test_journal_dedup_memory_and_oom(self):
            report = load_runner(REPORT_SOURCE)
            text = ('1000 host systemd[1]: neomacs-slot-1-900000000000.scope: Consumed 2s CPU time over 1min 0s wall clock time, 8G memory peak.\n')
            jobs, oom = report.journal_jobs(text + text + '1001 host systemd[1]: neomacs-slot-1-900000000000.scope: Failed with result oom-kill.\n')
            self.assertEqual(len(jobs), 1)
            row = next(iter(jobs.values()))
            self.assertAlmostEqual(row['wait'], 40 / 60)
            self.assertEqual(row['wall'], 1)
            self.assertEqual(row['peak'], 8)
            self.assertEqual(len(oom), 1)

        def test_group_prefix_and_oom_without_hol(self):
            report = load_runner(REPORT_SOURCE)
            ledger = [dict(unit='u', group='quick-one', finished=350)]
            rows = report.summarize({'u': dict(wait=3, wall=1, peak=8)}, {'u'}, ledger, {}, 0, 400)
            self.assertEqual([r['group'] for r in rows], ['ALL', 'quick'])
            self.assertEqual(rows[0]['oom'], 1)
            self.assertEqual(rows[1]['oom'], 1)

        def test_m3_copied_baseline_is_one_execution(self):
            import shutil
            report = load_runner(REPORT_SOURCE)
            with tempfile.TemporaryDirectory(prefix='velocity-copy-', dir=os.environ['TMPDIR']) as directory:
                root = Path(directory)
                runs, baselines = root / 'runs', root / 'baselines'
                run = runs / 'run-id'
                run.mkdir(parents=True)
                baselines.mkdir()
                for name, text, stamp in [('cargo-metadata.json', '{}', 1000),
                        ('core.jsonl', '{"type":"test","event":"ok"}\n', 1060),
                        ('failures.txt', '', 1060)]:
                    path = run / name
                    path.write_text(text)
                    os.utime(path, (stamp, stamp))
                shutil.copytree(run, baselines / 'commit-id')
                rows = report.gate_runs([runs, baselines], 900, 1200)
                self.assertEqual(len(rows), 1, 'copied baseline counted as another execution')
                # Identical results from a separate execution must still count.
                other = runs / 'other-run'
                shutil.copytree(run, other)
                for path in other.iterdir():
                    stamp = path.stat().st_mtime + 100
                    os.utime(path, (stamp, stamp))
                self.assertEqual(len(report.gate_runs([baselines, runs], 900, 1200)), 2)

        def test_m5_s1_report_does_not_claim_hol(self):
            report = load_runner(REPORT_SOURCE)
            ledger = [dict(unit=u, group='quick-one', finished=400, hol_spans=spans)
                      for u, spans in [('a', [[100, 300]]), ('b', [[100, 300]]),
                                       ('c', [[200, 300]])]]
            # S1 drops HOL entirely, including stale overlapping/day-crossing snapshots.
            for start, end in [(0, 600), (150, 450)]:
                rows = report.summarize({}, set(), ledger, {}, start, end)
                self.assertTrue(all('hol_minutes' not in row for row in rows))

        def test_gate_cohort_zero_and_wasted_display(self):
            report = load_runner(REPORT_SOURCE)
            with tempfile.TemporaryDirectory(prefix='velocity-gates-', dir=os.environ['TMPDIR']) as directory:
                root = Path(directory)
                for name, n, display_tests in [('full', 100, 1), ('filtered', 1, 0), ('empty', 0, 0)]:
                    run = root / name
                    run.mkdir()
                    for filename, text in [('cargo-metadata.json', '{}'), ('failures.txt', ''),
                            ('display-binaries.json', '{}'),
                            ('core.jsonl', ''.join(json.dumps(dict(type='test', event='ok')) + '\n' for _ in range(n))),
                            ('display.jsonl', ''.join(json.dumps(dict(type='test', event='ok')) + '\n' for _ in range(display_tests)))]:
                        path = run / filename
                        path.write_text(text)
                        timestamp = 1000 if filename == 'cargo-metadata.json' else 1060
                        os.utime(path, (timestamp, timestamp))
                rows = report.gate_runs([root, root], 900, 1100)
                self.assertEqual(len(rows), 3)
                self.assertEqual({r['scope'] for r in rows}, {'full', 'filtered', 'no-tests'})
                self.assertTrue(all(r['seconds'] == 60 for r in rows))
                self.assertEqual(sum(r['wasted_display'] for r in rows), 2)
                self.assertEqual(sum(r['complete'] and not sum(r['tests'].values()) for r in rows), 1)

        def test_read_only_cli_missing_ledger_and_empty_data(self):
            with tempfile.TemporaryDirectory(prefix='velocity-report-', dir=os.environ['TMPDIR']) as directory:
                root = Path(directory)
                journal = root / 'journal'
                journal.write_text('')
                before = [(str(p), p.stat().st_mtime_ns, p.stat().st_size) for p in root.rglob('*')]
                result = subprocess.run([sys.executable, str(REPORT_SOURCE),
                    '--date', '2026-10-13', '--journal', str(journal), '--ledger', str(root / 'absent'),
                    '--gate-root', str(root / 'gates'), '--baseline-root', str(root / 'baseline'),
                    '--transcripts', str(root / 'logs'), '--json'], text=True, capture_output=True, check=True)
                data = json.loads(result.stdout)
                self.assertEqual(data['slots'][0]['count'], 0)
                self.assertIsNone(data['slots'][0]['wait_p50'])
                self.assertNotIn('hol_minutes', data['slots'][0])
                self.assertNotIn('hol_jobs', data)
                self.assertEqual(data['gates'], [])
                self.assertEqual(before, [(str(p), p.stat().st_mtime_ns, p.stat().st_size) for p in root.rglob('*')])

        def test_torn_ledger_and_dead_pid_live_scope_compatibility(self):
            report = load_runner(REPORT_SOURCE)
            with tempfile.TemporaryDirectory(prefix='velocity-records-', dir=os.environ['TMPDIR']) as directory:
                root = Path(directory)
                ledger = root / 'jobs.jsonl'
                ledger.write_text('{"seq": 1}\n{"seq":')
                self.assertEqual(report.ledger_rows(ledger), [{'seq': 1}])

    class RunnerCompatibilityTests(unittest.TestCase):
        def test_dead_scheduler_live_scope_is_retained(self):
            with tempfile.TemporaryDirectory(prefix='slot-compat-', dir=os.environ['TMPDIR']) as directory:
                root = Path(directory)
                ticket = root / '00000000000000000001.json'
                ticket.write_text(json.dumps(dict(pid=99999999, start='0', active=True, unit='owned')))
                for source in (OLD_SOURCE, SOURCE):
                    module = load_runner(source)
                    module.STATE = root
                    module.alive = lambda _: False  # Model only; never probe an unrelated PID.
                    module.scope_alive = lambda _: True
                    self.assertEqual(len(module.records()), 1)
                    self.assertTrue(ticket.exists())
                    module.scope_alive = lambda _: False
                    self.assertEqual(module.records(), [])
                    ticket.write_text(json.dumps(dict(pid=99999999, start='0', active=True, unit='owned')))

    unittest.main(argv=[sys.argv[0], *sys.argv[2:]])

if len(sys.argv) > 1 and sys.argv[1] == 'child':
    root, lane, duration = sys.argv[2:]
    module = load_runner()
    module.STATE = Path(root) / 'queue'
    module.LEGACY = Path(root)
    module.Path.home = classmethod(lambda cls: Path(root) / 'home')
    module.shutil.which = lambda _: '/bin/true'
    module.systemctl = lambda *a: subprocess.CompletedProcess(a, 0, 'ActiveState=inactive\nLoadState=not-found\n', '')
    original = module.subprocess.call
    def payload(argv, **kwargs):
        if argv[0] == 'systemd-run':
            assert kwargs['env']['SCCACHE_CLIENT_SIDE'] == '1'
            assert 'SCCACHE_LOG' not in kwargs['env'] and 'SCCACHE_ERROR_LOG' not in kwargs['env']
            assert kwargs['env']['RUST_TEST_THREADS'] == kwargs['env']['NEXTEST_TEST_THREADS']
            if lane.startswith(('perf-', 'full-')):
                assert 'CARGO_PROFILE_TEST_DEBUG' not in kwargs['env']
                assert 'CARGO_PROFILE_RELEASE_LTO' not in kwargs['env']
            else:
                # The installed runner no longer overrides debug info; preserve
                # that baseline while retaining the lower-cost LTO assertion.
                assert 'CARGO_PROFILE_TEST_DEBUG' not in kwargs['env']
                assert kwargs['env']['CARGO_PROFILE_RELEASE_LTO'] == 'off'
            with open(Path(root) / 'events', 'a') as stream:
                stream.write(json.dumps([lane, 'start', time.monotonic(), kwargs['env']['CARGO_TARGET_DIR'], kwargs['env']['CARGO_BUILD_JOBS']]) + '\n')
            time.sleep(float(duration))
            with open(Path(root) / 'events', 'a') as stream:
                stream.write(json.dumps([lane, 'end', time.monotonic()]) + '\n')
            return 7 if lane == 'error-one' else 0
        return original(argv, **kwargs)
    module.subprocess.call = payload
    save = module.save
    def registered(path, ticket):
        save(path, ticket)
        fd = os.environ.pop('NEOMACS_SLOT_TEST_READY_FD', None)
        if fd is not None:
            os.write(int(fd), b'1')
            os.close(int(fd))
    module.save = registered
    sys.argv = [str(SOURCE), lane, '--mem', '1M', '--slots', '2', '--test-build']
    if lane.startswith('full-'):
        sys.argv.append('--profile-build')
    sys.argv.extend(['--', 'true'])
    sys.exit(module.main())

spec = importlib.util.spec_from_file_location('runner', SOURCE, loader=SourceFileLoader('runner', str(SOURCE)))
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.systemctl = lambda *a: subprocess.CompletedProcess(a, 1, '', 'bus error')
assert module.scope_alive('owned-test') is True
module.systemctl = lambda *a: subprocess.CompletedProcess(a, 0, 'ActiveState=inactive\nLoadState=not-found\n', '')
assert module.scope_alive('owned-test') is False

with tempfile.TemporaryDirectory(prefix='neomacs-slot-test-', dir=os.environ['TMPDIR']) as directory:
    root = Path(directory)
    # Modeled scheduler descriptor loss: payload holds its own shared locks.
    import fcntl
    bridges = [(root / f'bridge.{n}').open('a') for n in (1, 2)]
    for bridge in bridges:
        fcntl.flock(bridge, fcntl.LOCK_SH)
    payload = subprocess.Popen(['flock', '--shared', str(root / 'bridge.1'), 'flock', '--shared', str(root / 'bridge.2'), 'sh', '-c', 'touch "$1"; sleep 1', 'sh', str(root / 'ready')])
    for _ in range(100):
        if (root / 'ready').exists():
            break
        time.sleep(.01)
    assert (root / 'ready').exists()
    for bridge in bridges:
        bridge.close()
    for n in (1, 2):
        assert subprocess.call(['flock', '--nonblock', '--exclusive', str(root / f'bridge.{n}'), 'true']) == 1
    assert payload.wait(timeout=5) == 0
    for n in (1, 2):
        assert subprocess.call(['flock', '--nonblock', '--exclusive', str(root / f'bridge.{n}'), 'true']) == 0
    children = []
    for lane, duration in [('alpha-one', '5'), ('alpha-two', '.2'), ('beta-one', '4'), ('error-one', '.1')]:
        import select
        ready, notify = os.pipe()
        try:
            children.append(subprocess.Popen([sys.executable, __file__, 'child', directory, lane, duration],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, pass_fds=(notify,),
                env={**os.environ, 'NEOMACS_SLOT_TEST_READY_FD': str(notify)}))
        finally:
            os.close(notify)
        try:
            assert select.select([ready], [], [], 10)[0], 'child did not register a ticket'
            assert os.read(ready, 1) == b'1'
        finally:
            os.close(ready)
    codes = [p.wait(timeout=30) for p in children]
    events = [json.loads(line) for line in (root / 'events').read_text().splitlines()]
    starts = [row for row in events if row[1] == 'start']
    assert [row[0] for row in starts] == ['alpha-one', 'beta-one', 'error-one', 'alpha-two'], events
    first_end = next(row[2] for row in events if row[:2] == ['alpha-one', 'end'])
    assert next(row[2] for row in starts if row[0] == 'alpha-two') >= first_end
    assert starts[0][3] == next(row[3] for row in starts if row[0] == 'alpha-two')
    assert all(row[4] == str(max(1, (os.cpu_count() or 1) // 2)) for row in starts)
    assert codes == [0, 0, 0, 7], codes
    assert not list((root / 'queue').glob('*.json'))
    for lane in ('perf-one', 'full-one'):
        assert subprocess.call([sys.executable, __file__, 'child', directory, lane, '.01']) == 0
    print(json.dumps(dict(result='PASS', exits=codes, events=events, profile_tests='PASS'), indent=2))
    subprocess.run([sys.executable, __file__, '--s1-tests', '-v'], check=True)
