"""Dual census (R210): WARM (bootstrap chain preloaded) and COLD (fresh VM).

Run: python3 -m unittest discover -s scripts -p 'test_lisp_census_modes.py' -v
Uses the measurement binary at $CENSUS_BIN, else the census target's release
build. Each fixture runs the real binary in a subprocess.
"""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('census', Path(__file__).with_name('lisp-census.py'))
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)
BINARY = Path(os.environ.get('CENSUS_BIN') or Path(os.environ.get('CENSUS_TARGET_DIR', str(next(p for p in _PortablePath(__file__).resolve().parents if (p / 'parity-reference.toml').is_file()) / 'target'))) / 'release/romacs-vm-census')
NOT_SPLIT = {'form_count': None, 'status': 'not_split'}


@unittest.skipUnless(BINARY.exists(), f'census binary missing: {BINARY}')
class DualCensusTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='census-modes-', dir=str(_PortablePath(_portable_tempfile.gettempdir()) / ''))
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.ok = self.write('chain-ok.el', '(defvar census-warm-dep 41)\n(defvar census-warm-ok t)\n')
        self.bad = self.write('chain-bad.el', '(defvar census-warm-partial 1)\n(census-warm-impossible)\n(defvar census-warm-after-failure t)\n')
        self.after = self.write('chain-after.el', '(defvar census-warm-after t)\n')
        self.chain = [self.ok, self.bad, self.after]

    def write(self, name, text):
        path = self.root / name
        path.write_text(text)
        return path.resolve()

    def run_row(self, target, preload=()):
        return census.run_file(target, NOT_SPLIT, BINARY, [self.root], 10, preload=preload)

    def test_warm_preloads_chain_until_first_failure_and_measures_only_target(self):
        target = self.write('target.el', '(setq census-target (+ census-warm-dep census-warm-partial))\n'
                            '(if (boundp (quote census-warm-after)) (error "chain did not stop"))\n'
                            '(if (boundp (quote census-warm-after-failure)) (error "failed form resumed"))\n')
        row = self.run_row(target, self.chain)
        self.assertEqual(row.get('mode'), 'warm', row)
        self.assertTrue(row['loaded'], row)
        self.assertEqual(row['forms_evaluated'], 3, row)
        self.assertEqual(row['dependency_forms_evaluated'], 0, 'chain steps must not count as dependencies')
        chain = row['chain']
        self.assertEqual([c['loaded'] for c in chain], [True, False], 'chain stops at its first failing file')
        self.assertEqual(chain[0]['forms_completed'], 2)
        self.assertEqual(chain[1]['forms_completed'], 1)
        self.assertEqual(chain[1]['frontier']['ordinal'], 2)
        self.assertEqual(chain[1]['frontier']['error'], '(void-function census-warm-impossible)')

    def test_warm_target_that_is_a_chain_file_is_counted_once(self):
        row = self.run_row(self.ok, self.chain)
        self.assertEqual(row.get('mode'), 'warm', row)
        self.assertTrue(row['loaded'], row)
        self.assertEqual(row['forms_evaluated'], 2, 'chain steps of the same file must be cut, not doubled')

    def test_cold_is_unchanged_and_says_so(self):
        target = self.write('target.el', '(setq census-target (+ census-warm-dep 1))\n')
        row = self.run_row(target)
        self.assertEqual(row.get('mode'), 'cold', row)
        self.assertNotIn('chain', row)
        self.assertFalse(row['loaded'])
        self.assertEqual(row['first_failure']['classification'], 'missing_variable')
        self.assertEqual(row['first_failure']['symbol'], 'census-warm-dep')

    def test_eager_target_is_installed_after_normal_chain_and_changes_loader_regime(self):
        chain = self.write('regime-chain.el', '(if (fboundp (quote internal-macroexpand-for-load)) (error "eager before bootstrap"))\n')
        target = self.write('regime-target.el', '(if (not (fboundp (quote internal-macroexpand-for-load))) (error "target not eager"))\n'
                            '(if (not (eq (internal-macroexpand-for-load (quote census-sentinel)) (quote census-sentinel))) (error "not identity"))\n')
        default = self.run_row(target, [chain])
        eager = census.run_file(target, NOT_SPLIT, BINARY, [self.root], 10, preload=[chain], eager_target=True)
        self.assertFalse(default['loaded'], default)
        self.assertTrue(eager['loaded'], eager)
        self.assertEqual(eager['forms_evaluated'], 2)
        self.assertEqual(eager['dependency_forms_evaluated'], 0)
        self.assertEqual(default['chain'], eager['chain'])
        self.assertTrue(eager['eager_target'])
        self.assertEqual(eager['measurement_regime'], 'eager_counterfactual')

    def test_eager_flag_changes_expansion_of_unreached_function_body(self):
        chain = self.write('macro-chain.el', '(defmacro census-eager-bomb () (error "eager expansion reached"))\n')
        target = self.write('unreached-body.el', '(defun census-unreached () (census-eager-bomb))\n')
        default = self.run_row(target, [chain])
        eager = census.run_file(target, NOT_SPLIT, BINARY, [self.root], 10, preload=[chain], eager_target=True)
        self.assertTrue(default['loaded'], default)
        self.assertFalse(eager['loaded'], eager)
        self.assertIn('eager expansion reached', eager['first_failure']['error'])
        self.assertEqual(default['chain'], eager['chain'])

    def test_preload_form_preserves_dynamic_binding_and_excludes_nested_counts(self):
        scoped = self.write('scoped.el', '(if (not (eq census-scope (quote skip))) (error "binding absent"))\n')
        target = self.write('target.el', '(if (not (eq census-scope (quote outside))) (error "binding leaked"))\n')
        form = f'(progn (defvar census-scope (quote outside)) (let ((census-scope (quote skip))) (load {json.dumps(str(scoped))})))'
        row = self.run_row(target, [form])
        self.assertTrue(row['loaded'], row)
        self.assertEqual(row['forms_evaluated'], 1)
        self.assertEqual(row['dependency_forms_evaluated'], 0)
        self.assertEqual(row['chain'][0]['form'], form)
        self.assertEqual(row['chain'][0]['forms_completed'], 1)
        self.assertEqual(row['chain'][0]['executed_loads'], [
            {'file': str(scoped), 'forms_completed': 1, 'failed': False}])

    def test_failed_nested_load_records_frontier_and_stops_chain(self):
        target = self.write('target.el', '(if (boundp (quote census-warm-after)) (error "continued"))\n')
        row = self.run_row(target, [f'(load {json.dumps(str(self.bad))})', self.after])
        self.assertTrue(row['loaded'], row)
        self.assertEqual(len(row['chain']), 1)
        entry = row['chain'][0]
        self.assertFalse(entry['loaded'])
        self.assertEqual(entry['forms_completed'], 1)
        self.assertEqual(entry['frontier']['file'], str(self.bad))
        self.assertEqual(entry['frontier']['ordinal'], 2)

    def test_untaken_form_branch_reports_no_executed_load(self):
        target = self.write('target.el', 't\n')
        row = self.run_row(target, [f'(if nil (load {json.dumps(str(self.bad))}))'])
        self.assertTrue(row['loaded'], row)
        self.assertTrue(row['chain'][0]['loaded'])
        self.assertEqual(row['chain'][0]['executed_loads'], [])
        self.assertEqual(row['chain'][0]['forms_completed'], 0)

    def test_failed_preload_form_stops_before_later_files(self):
        target = self.write('target.el', '(if (boundp (quote census-warm-after)) (error "continued after failure"))\n')
        row = self.run_row(target, ['(error "preload failed")', self.after])
        self.assertTrue(row['loaded'], row)
        self.assertEqual(len(row['chain']), 1)
        self.assertFalse(row['chain'][0]['loaded'])
        self.assertIn('preload failed', row['chain'][0]['error'])

    def test_chain_must_be_identical_across_rows(self):
        a = {'file': 'a', 'forms_evaluated': 1, 'chain': [{'file': 'x', 'loaded': True, 'forms_completed': 2}]}
        b = {'file': 'b', 'forms_evaluated': 1, 'chain': [{'file': 'x', 'loaded': False, 'forms_completed': 1}]}
        self.assertEqual(census.chain_consensus([a, dict(a, file='c')]), a['chain'])
        with self.assertRaises(RuntimeError):
            census.chain_consensus([a, b])


class ChainDefinitionTests(unittest.TestCase):
    def test_chain_follows_gnu_loadup_order_and_every_link_exists(self):
        chain = census.bootstrap_chain()
        paths = [p for p in chain if isinstance(p, Path)]
        names = [str(p.relative_to(census.ROOT / 'lisp')) for p in paths]
        self.assertEqual(names[:4], ['emacs-lisp/debug-early.el', 'emacs-lisp/byte-run.el', 'emacs-lisp/backquote.el', 'subr.el'])
        for path in paths:
            self.assertTrue(path.is_file(), path)
        self.assertEqual(chain[-1], census.macroexp_bootstrap_form())
        self.assertIn("(let ((macroexp--pending-eager-loads '(skip)))", chain[-1])
        self.assertIn('(load "emacs-lisp/pcase")', chain[-1])
        self.assertIn('(load "emacs-lisp/macroexp")', chain[-1])
        self.assertLess(names.index('window.el'), names.index('emacs-lisp/macroexp.el'))
        self.assertLess(names.index('files.el'), names.index('emacs-lisp/macroexp.el'))
        loadup = (census.ROOT / 'lisp/loadup.el').read_text()
        positions = [loadup.index(f'(load "{n[:-3]}")') for n in names]
        self.assertEqual(positions, sorted(positions), 'chain must keep loadup.el order')


if __name__ == '__main__':
    unittest.main()
