"""Focused harness tests; no Cargo build or ambient compositor required."""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import live_oracle as harness


class HarnessTests(unittest.TestCase):
    def test_scene_geometry_and_mutation(self):
        metrics = {"ascent": 17, "descent": 6, "cell_width": 12}
        before = harness.scene(Path('/font.ttf'), metrics, after=False)
        after = harness.scene(Path('/font.ttf'), metrics, after=True)
        self.assertEqual(before['frame']['width'], 400)
        self.assertEqual(before['frame']['height'], 300)
        self.assertEqual(before['expect']['origin'], [0, 0])
        self.assertEqual(before['expect']['rows'][0]['text'], 'Ωμέγα Hello')
        self.assertEqual(after['expect']['rows'][1]['text'], 'Δ')
        self.assertIsNone(after['expect']['cursor'])
        self.assertEqual(after['font']['ascent'], 17)
        self.assertEqual(after['expect']['rows'][0]['runs'],
                         [{'start': 6, 'end': 11, 'fg': '#ff0000'}])

    def test_independent_font_metrics(self):
        font = Path('/usr/share/fonts/liberation/LiberationMono-Regular.ttf')
        self.assertEqual(harness.font_metrics(font, 'magick'),
                         {'ascent': 17, 'descent': 6, 'cell_width': 12})

    def test_reply_reader_retains_coalesced_lines(self):
        proc = subprocess.Popen([sys.executable, '-c',
                                 'import os; os.write(1, b\'{"ready":true,"presentation":1}\\n'
                                 '{"result":"nil","presentation":2}\\n\')'],
                                stdout=subprocess.PIPE)
        try:
            reader = harness.ReplyReader(proc, timeout=2)
            self.assertTrue(reader.read()['ready'])
            self.assertEqual(reader.read()['presentation'], 2)
            with self.assertRaises(RuntimeError):
                reader.read()
        finally:
            proc.wait(timeout=2)
            proc.stdout.close()

    def test_reply_rejects_non_object(self):
        proc = subprocess.Popen([sys.executable, '-c', 'print("[]")'],
                                stdout=subprocess.PIPE)
        try:
            with self.assertRaises(ValueError):
                harness.ReplyReader(proc, timeout=2).read()
        finally:
            proc.wait(timeout=2)
            proc.stdout.close()

    def test_mutations_preserve_shape_and_remove_delta(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).parent) as temp:
            root = Path(temp)
            before = root / 'before.png'
            after = root / 'after.png'
            subprocess.run(['magick', '-size', '400x300', 'xc:black', '-fill', 'red',
                            '-draw', 'rectangle 72,3 77,14', str(before)], check=True)
            subprocess.run(['magick', str(before), '-fill', 'white', '-draw',
                            'rectangle 2,26 9,38', str(after)], check=True)
            mutants = harness.make_mutants(before, after, root, 'magick',
                                           {'cell_width': 12, 'ascent': 17, 'descent': 6})
            self.assertEqual(set(mutants), {'wrong-colour', 'shift-1px', 'delta-absent'})
            self.assertEqual(harness.rgb(mutants['delta-absent'], 'magick'),
                             harness.rgb(before, 'magick'))
            wrong = harness.rgb(mutants['wrong-colour'], 'magick')
            self.assertEqual(wrong[(3 * 400 + 72) * 3: (3 * 400 + 72) * 3 + 3], b'\x00\xff\x00')
            shifted = harness.rgb(mutants['shift-1px'], 'magick')
            self.assertEqual(shifted[(3 * 400 + 73) * 3: (3 * 400 + 73) * 3 + 3], b'\xff\x00\x00')

    @unittest.skipUnless(os.environ.get('G1A_ORACLE'), 'set G1A_ORACLE for real oracle checks')
    def test_real_oracle_on_explicitly_synthetic_font_render(self):
        # This tests the judge wiring and mutants, NOT live GUI acceptance.
        oracle = os.environ['G1A_ORACLE']
        font = Path('/usr/share/fonts/liberation/LiberationMono-Regular.ttf')
        metrics = harness.font_metrics(font, 'magick')
        with tempfile.TemporaryDirectory(dir=Path(__file__).parent) as temp:
            root = Path(temp)
            before, after = root / 'before.png', root / 'after.png'
            subprocess.run(['magick', '-density', '72', '-size', '400x300', 'xc:black',
                            '-font', str(font), '-pointsize', '20', '-fill', 'white',
                            '-annotate', '+0+17', 'Ωμέγα ', '-fill', 'red',
                            '-annotate', '+72+17', 'Hello', str(before)], check=True)
            subprocess.run(['magick', str(before), '-density', '72', '-font', str(font),
                            '-pointsize', '20', '-fill', 'white', '-annotate', '+0+40',
                            'Δ', str(after)], check=True)
            for phase, image in [('before', before), ('after', after)]:
                spec = root / f'{phase}.json'
                harness.save_json(spec, harness.scene(font, metrics, after=phase == 'after'))
                harness.judge(oracle, spec, image, root / f'{phase}-check.json', reject=False)
            for kind, image in harness.make_mutants(before, after, root, 'magick', metrics).items():
                harness.judge(oracle, root / 'after.json', image,
                              root / f'{kind}-check.json', reject=True)


if __name__ == '__main__':
    unittest.main()
