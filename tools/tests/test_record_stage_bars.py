import importlib.machinery
import importlib.util
import json
from pathlib import Path
import shutil
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
                   'PATH': '/usr/bin:/bin', 'XDG_RUNTIME_DIR': '/run/user/live'}
            with patch.object(recorder.subprocess, 'run') as run:
                recorder.measure('display', work / 'bars.json', work / 'target', work, env)
            child = run.call_args.kwargs['env']
            self.assertEqual(shutil.which('wlr-randr', path=child['PATH']), str(randr))
            self.assertEqual(child['PATH'].split(':')[0], env['ROMACS_TOOLCHAIN_BIN'])
            self.assertEqual(child['XDG_RUNTIME_DIR'], env['XDG_RUNTIME_DIR'])


class Publication(unittest.TestCase):
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
