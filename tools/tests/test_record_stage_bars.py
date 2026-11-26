import importlib.machinery
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'record-stage-bars'
LOADER = importlib.machinery.SourceFileLoader('record_stage_bars', str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
recorder = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(recorder)


class Measurement(unittest.TestCase):
    def test_harness_finds_randr_beside_the_pinned_compositor(self):
        with tempfile.TemporaryDirectory() as scratch:
            work = Path(scratch)
            headless = work / 'headless'
            headless.mkdir()
            randr = headless / 'wlr-randr'
            randr.touch(mode=0o700)
            env = {'ROMACS_TOOLCHAIN_BIN': str(work / 'rust'),
                   'ROMACS_TEST_LABWC': str(headless / 'labwc'),
                   'PATH': '/usr/bin:/bin', 'XDG_RUNTIME_DIR': '/run/user/live',
                   'CARGO_INCREMENTAL': '1'}
            with patch.object(recorder.subprocess, 'run') as run:
                recorder.measure('display', work / 'bars.json', work / 'target', work, env)
            child = run.call_args.kwargs['env']
            self.assertEqual(shutil.which('wlr-randr', path=child['PATH']), str(randr))
            self.assertEqual(child['PATH'].split(':')[0], env['ROMACS_TOOLCHAIN_BIN'])
            self.assertEqual(child['XDG_RUNTIME_DIR'], env['XDG_RUNTIME_DIR'])
            self.assertEqual(run.call_args_list[0].kwargs['env']['CARGO_INCREMENTAL'], '0')

    def test_retry_keeps_only_the_passing_draft(self):
        with tempfile.TemporaryDirectory() as scratch:
            work = Path(scratch)
            bars = work / 'bars.json'
            bars.write_text('{}\n')
            env = {'ROMACS_TOOLCHAIN_BIN': '/rust', 'ROMACS_TEST_LABWC': '/headless/labwc'}
            drafts = []

            def run(command, **_):
                if '--write-bars' not in command:
                    return
                draft = Path(command[command.index('--write-bars') + 1])
                drafts.append(draft)
                draft.write_text(f'{{"attempt": {len(drafts)}}}')
                if len(drafts) == 1:
                    raise subprocess.CalledProcessError(1, command)

            with patch.object(recorder.subprocess, 'run', side_effect=run) as spawn:
                draft = recorder.measure('display', bars, work / 'target', work, env)

            self.assertEqual(spawn.call_count, 3)
            self.assertNotEqual(drafts[0], drafts[1])
            self.assertEqual(draft.read_text(), '{"attempt": 2}')
            self.assertEqual(bars.read_text(), '{}\n')

    def test_retry_is_bounded(self):
        with tempfile.TemporaryDirectory() as scratch:
            work = Path(scratch)
            env = {'ROMACS_TOOLCHAIN_BIN': '/rust', 'ROMACS_TEST_LABWC': '/headless/labwc'}
            failure = subprocess.CalledProcessError(1, 'harness')
            with patch.object(recorder.subprocess, 'run', side_effect=[None, failure, failure]) as spawn:
                with self.assertRaises(subprocess.CalledProcessError):
                    recorder.measure('render', work / 'bars.json', work / 'target', work, env)
            self.assertEqual(spawn.call_count, 3)


class Publication(unittest.TestCase):
    def test_changed_snapshot_keeps_existing_workload_and_metric_order(self):
        with tempfile.TemporaryDirectory() as scratch:
            current = Path(scratch) / 'bars.json'
            draft = Path(scratch) / 'draft.json'
            current.write_text('{"first":{"count_note":"why","count":5},"second":{"count":5}}')
            draft.write_text('{"second":{"count":5},"third":{"count":1},"first":{"count":4,"new":1}}')
            self.assertTrue(recorder.publish(current, draft, 'display'))
            updated = json.loads(current.read_text())
            self.assertEqual(list(updated), ['first', 'second', 'third'])
            self.assertEqual(list(updated['first']), ['count_note', 'count', 'new'])
            self.assertEqual(updated['first'], {'count_note': 'why', 'count': 4, 'new': 1})

    def test_tightens_cost_and_reuse_without_changing_coverage(self):
        current = {'typed': {'rows_reused': 36, 'glyphs_walked': 228, 'own_instructions': 1_000,
                             'extra_frames': 'report', 'own_instructions_note': 'why'}}
        draft = {'typed': {'rows_reused': 37, 'glyphs_walked': 200, 'own_instructions': 1_001,
                           'extra_frames': 0, 'bytes_uploaded': 80_000}}
        result = recorder.tightened(current, draft, 'render')
        self.assertEqual(result, {'typed': {'rows_reused': 37, 'glyphs_walked': 200, 'own_instructions': 1_000,
                                            'extra_frames': 'report', 'own_instructions_note': 'why',
                                            'bytes_uploaded': 80_000}})

    def test_an_unchanged_snapshot_retains_its_bytes(self):
        with tempfile.TemporaryDirectory() as scratch:
            current = Path(scratch) / 'bars.json'
            draft = Path(scratch) / 'draft.json'
            before = '{"typed": {"windows_rebuilt": 2, "step_instructions": 100}}\n'
            current.write_text(before)
            draft.write_text('{"typed":{"step_instructions":109,"windows_rebuilt":2}}')
            self.assertFalse(recorder.publish(current, draft, 'display'))
            self.assertEqual(current.read_text(), before)
            self.assertCountEqual(Path(scratch).iterdir(), [current, draft])

    def test_a_new_metric_and_tighter_limit_replace_just_the_snapshot(self):
        with tempfile.TemporaryDirectory() as scratch:
            current = Path(scratch) / 'bars.json'
            draft = Path(scratch) / 'draft.json'
            current.write_text('{"typed":{"windows_rebuilt":2,"step_instructions":100}}')
            draft.write_text('{"typed":{"step_instructions":109,"windows_rebuilt":1,"rows_wrapped":0}}')
            self.assertTrue(recorder.publish(current, draft, 'display'))
            self.assertEqual(json.loads(current.read_text()),
                             {'typed': {'windows_rebuilt': 1, 'step_instructions': 100, 'rows_wrapped': 0}})
            self.assertCountEqual(Path(scratch).iterdir(), [current, draft])
