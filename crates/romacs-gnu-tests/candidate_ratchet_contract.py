#!/usr/bin/env python3
"""Fast CLI mutants for candidate monotonicity, without executing an oracle."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('parity_ratchet', ROOT / 'tools/lib/parity_ratchet.py')
assert SPEC is not None and SPEC.loader is not None
PR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PR)


class CandidateContract(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='candidate-ratchet-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.original = json.loads((ROOT / PR.RATCHET_NAME).read_text())
        self.data = copy.deepcopy(self.original)
        self.path = self.root / PR.RATCHET_NAME
        self.path.parent.mkdir()
        self.write()
        for name in PR.GATE_TESTS:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        self.git('init', '--quiet', '--initial-branch=master')
        self.git('add', '.')
        self.git('-c', 'user.name=Contract', '-c', 'user.email=contract@example.invalid',
                 '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'fixture')
        self.base = self.git('rev-parse', 'HEAD').strip()

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.root), *args], text=True)

    def write(self):
        self.path.write_text(json.dumps(self.data, sort_keys=True) + '\n')

    def run_cli(self, *args):
        env = dict(os.environ, ROMACS_MEMBER_EVIDENCE=str(self.root / 'absent-evidence'),
                   ROMACS_VACUITY_RECEIPT=str(self.root / 'absent-receipt'))
        return subprocess.run([sys.executable, str(ROOT / 'tools/lib/parity_ratchet.py'),
                               *args, '--root', str(self.root)], env=env, text=True, capture_output=True)

    def candidate(self):
        return self.run_cli('verify', '--candidate', '--base', self.base)

    def assert_rejected(self, diagnostic):
        self.assertNotEqual(self.data, self.original, 'mutant was not applied')
        self.write()
        result = self.candidate()
        self.assertEqual(result.returncode, PR.EXIT_LOWER, result.stdout + result.stderr)
        self.assertIn(diagnostic, result.stderr)

    def test_identical_static_summary_twice_without_evidence(self):
        before = self.path.read_bytes()
        first, second = self.candidate(), self.candidate()
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual((first.stdout, first.stderr), (second.stdout, second.stderr))
        self.assertIn('static, not a measurement', first.stdout)
        self.assertEqual(self.path.read_bytes(), before)
        self.assertFalse((self.root / 'absent-evidence').exists())
        print(first.stdout.strip())
        print(second.stdout.strip())

    def test_member_loss_hidden_by_equal_count_is_rejected(self):
        g = self.data['gnu_suite']
        f = next(f for f, members in g['passing'].items() if members)
        old = g['passing'][f][0]
        replacement = 'candidate-mutant-replacement'
        g['passing'][f] = sorted([replacement, *g['passing'][f][1:]])
        if old in g['vacuous_budget'].get(f, []):
            g['vacuous_budget'][f] = sorted(replacement if n == old else n for n in g['vacuous_budget'][f])
        g['classes'][f'{f}::{replacement}'] = g['classes'].pop(f'{f}::{old}')
        real = g['real_ert']['passing']
        if old in real.get(f, []):
            real[f] = sorted(replacement if n == old else n for n in real[f])
        self.assertEqual(PR.consistency(self.data), [])
        self.assert_rejected(old)

    def test_floor_lowering_with_authorization_is_rejected(self):
        old = self.data['census']['warm']['loaded'].pop()
        self.data['census']['warm']['gnu_parity_loaded'] -= 1
        self.data['lowerings'].append({'unit': 'census-warm', 'members': [old],
                                      'because': 'This mutant must never authorize a candidate lowering'})
        self.assertEqual(PR.consistency(self.data), [])
        self.assert_rejected('no floor lowering is permitted')

    def test_numeric_floor_edit_is_rejected(self):
        self.data['census']['cold']['gnu_parity_loaded'] -= 1
        self.assertNotEqual(self.data, self.original)
        self.write()
        result = self.candidate()
        self.assertEqual(result.returncode, PR.EXIT_INVALID, result.stderr)
        self.assertIn('gnu_parity_loaded', result.stderr)

    def test_denominator_lowering_is_rejected(self):
        self.data['census']['cold']['files_measured'] -= 1
        self.assertEqual(PR.consistency(self.data), [])
        self.assert_rejected('files_measured lowered')

    def test_committed_loss_restored_in_worktree_is_rejected(self):
        old = self.data['census']['cold']['loaded'].pop()
        self.data['census']['cold']['gnu_parity_loaded'] -= 1
        self.write()
        self.git('add', PR.RATCHET_NAME)
        self.git('-c', 'user.name=Contract', '-c', 'user.email=contract@example.invalid',
                 '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'mutant')
        self.data = copy.deepcopy(self.original)
        self.write()
        result = self.candidate()
        self.assertEqual(result.returncode, PR.EXIT_LOWER, result.stderr)
        self.assertIn(old, result.stderr)

    def rebuild_suite(self):
        g = self.data['gnu_suite']
        self.data['gnu_suite'] = PR.honest_suite(
            g['files'], g['tests_per_file'], g['passing'], g['classes'], g['real_ert'])
        self.assertEqual(PR.consistency(self.data), [])

    def test_post_land_removed_before_any_build_or_probe(self):
        help_result = self.run_cli('record', '--help')
        self.assertEqual(help_result.returncode, 0, help_result.stderr)
        self.assertIn('--out', help_result.stdout)
        self.assertNotIn('--post-land', help_result.stdout)
        self.git('checkout', '--quiet', '-b', 'candidate')
        before = self.path.read_bytes()
        result = self.run_cli('record', '--post-land')
        self.assertEqual(result.returncode, PR.EXIT_INVALID, result.stderr)
        self.assertIn('unrecognized arguments: --post-land', result.stderr)
        self.assertEqual(self.path.read_bytes(), before)

    def test_budget_promotion_to_genuine_is_accepted(self):
        g = self.data['gnu_suite']
        f, name = next((f, n) for f, names in g['vacuous_budget'].items() for n in names)
        g['classes'][f'{f}::{name}'].update(
            {'class': 'GENUINE', 'counted': True, 'file_loaded': True, 'failed_forms': [],
             'vacuous_catch': []})
        self.rebuild_suite()
        self.assertNotEqual(self.data, self.original)
        self.assertNotIn(name, self.data['gnu_suite']['vacuous_budget'].get(f, []))
        self.write()
        result = self.candidate()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_budget_growth_is_rejected(self):
        g = self.data['gnu_suite']
        key = next(k for k, c in g['classes'].items() if c['class'] == 'UNPROBEABLE')
        g['classes'][key]['class'] = 'VACUOUS'
        self.rebuild_suite()
        self.assert_rejected('vacuous budget may only decrease')

    def test_budget_member_loss_still_violates_all_pass_floor(self):
        g = self.data['gnu_suite']
        f, name = next((f, n) for f, names in g['vacuous_budget'].items() for n in names
                       if n not in g['real_ert']['passing'].get(f, []))
        g['passing'][f].remove(name)
        del g['classes'][f'{f}::{name}']
        g['real_ert']['of'] -= 1
        self.rebuild_suite()
        self.assert_rejected(f'all:{f}: 1 dropped ({name})')


if __name__ == '__main__':
    unittest.main()
