import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import package_parity as pp

MANIFEST = '''
[package.a]
url = "https://example.invalid/a"
rev = ""
deps = ["b"]
tests = ["test/*.el"]

[package.b]
url = "https://example.invalid/b"
rev = "old"
main_file = "b.el"
ignored_files = ["b-extra.el"]
'''


def ert(*events):
    return '\n'.join('ERT-SCOREBOARD ' + json.dumps(e) for e in events)


class Manifest(unittest.TestCase):
    def test_deps_install_first(self):
        packages = pp.load_manifest(MANIFEST)
        self.assertEqual(pp.install_order(packages, ['a']), ['b', 'a'])

    def test_unknown_dep_and_key_refused(self):
        with self.assertRaises(ValueError):
            pp.load_manifest('[package.a]\nurl = "u"\ndeps = ["zz"]\n')
        with self.assertRaises(ValueError):
            pp.load_manifest('[package.a]\nurl = "u"\nbogus = 1\n')

    def test_cycle_refused(self):
        packages = pp.load_manifest('[package.a]\nurl="u"\ndeps=["b"]\n[package.b]\nurl="v"\ndeps=["a"]\n')
        with self.assertRaises(ValueError):
            pp.install_order(packages)

    def test_set_revisions_touches_only_named_blocks(self):
        text = pp.set_revisions(MANIFEST, {'b': 'new'})
        self.assertIn('rev = "new"', text)
        self.assertEqual(pp.load_manifest(text)['a']['rev'], '')

    def test_vc_spec(self):
        b = pp.load_manifest(MANIFEST)['b']
        self.assertEqual(pp.vc_spec(b, '/m/b.git'),
                         '\'(b :url "/m/b.git" :main-file "b.el" :ignored-files ("b-extra.el"))')


class Results(unittest.TestCase):
    def test_complete_file(self):
        output = ert({'event': 'selected', 'names': ['t1', 't2', 't3']},
                     {'event': 'result', 'test': 't1', 'outcome': 'passed', 'as_expected': True},
                     {'event': 'result', 'test': 't2', 'outcome': 'failed', 'as_expected': False},
                     {'event': 'result', 'test': 't3', 'outcome': 'failed', 'as_expected': True},
                     {'event': 'done'})
        result = pp.ert_result(output, 1)
        self.assertEqual(result['status'], 'complete')
        self.assertEqual(result['tests'], {'t1': 'passed', 't2': 'failed', 't3': 'expected-failure'})

    def test_signal_after_done_is_not_complete(self):
        output = ert({'event': 'selected', 'names': ['t']},
                     {'event': 'result', 'test': 't', 'outcome': 'passed', 'as_expected': True},
                     {'event': 'done'})
        self.assertEqual(pp.ert_result(output, 1)['status'], 'complete')
        self.assertEqual(pp.ert_result(output, -11)['status'], 'exit -11')
        self.assertEqual(pp.process_state(0, True), 'complete')
        self.assertEqual(pp.process_state(-9, True), 'exit -9')
        self.assertEqual(pp.process_state(None, True), 'timeout')

    def test_states_without_done(self):
        self.assertEqual(pp.ert_result('Error: void-function (x)', 255)['status'], 'load-error')
        self.assertEqual(pp.ert_result(ert({'event': 'selected', 'names': ['t']}), 255)['status'], 'aborted')
        self.assertEqual(pp.ert_result('', None)['status'], 'timeout')

    def package(self, romacs_tests, gnu_tests, steps=({'name': 's', 'value': '1'},)):
        def side(tests, value):
            return dict(install=dict(ok=True),
                        tests=dict(files={'t.el': dict(status='complete', tests=tests)}),
                        journey=dict(status='complete', steps=[dict(s, value=value) for s in steps]))
        return dict(romacs=side(romacs_tests, '1'), gnu=side(gnu_tests, '1'))

    def test_first_divergence_names_the_test(self):
        package = self.package({'a': 'passed', 'b': 'failed'}, {'a': 'passed', 'b': 'passed'})
        self.assertEqual(pp.first_divergence(package), 't.el: b: romacs failed, gnu passed')
        self.assertIsNone(pp.first_divergence(self.package({'a': 'passed'}, {'a': 'passed'})))

    def test_incomplete_journeys_are_not_same(self):
        package = self.package({'a': 'passed'}, {'a': 'passed'})
        for side in ('romacs', 'gnu'):
            package[side]['journey']['status'] = 'timeout'
        self.assertEqual(pp.journey_state(package), 'incomplete')
        self.assertIn('journey: romacs timeout', pp.first_divergence(package))

    def test_file_in_one_checkout_diverges(self):
        package = self.package({'a': 'passed'}, {'a': 'passed'})
        package['gnu']['tests']['files']['u.el'] = dict(status='complete', tests={})
        self.assertEqual(pp.first_divergence(package), "u.el: only in gnu's checkout")

    def test_regressions_cover_completion_and_expected_failures(self):
        old = dict(packages={'p': self.package({'a': 'expected-failure'}, {'a': 'expected-failure'})})
        new = dict(packages={'p': self.package({'a': 'expected-failure'}, {'a': 'expected-failure'})})
        new['packages']['p']['romacs']['tests']['files']['t.el']['status'] = 'timeout'
        self.assertEqual(pp.regressions(old, new), ['p: t.el: complete, now timeout'])
        new = dict(packages={'p': self.package({'a': 'failed'}, {'a': 'expected-failure'})})
        self.assertEqual(pp.regressions(old, new), ['p: t.el: a: expected-failure, now failed'])

    def test_regressions(self):
        old = dict(packages={'p': self.package({'a': 'passed'}, {'a': 'passed'})})
        new = dict(packages={'p': self.package({'a': 'failed'}, {'a': 'passed'})})
        self.assertEqual(pp.regressions(old, new), ['p: t.el: a: passed, now failed'])
        self.assertEqual(pp.regressions(new, old), [])


if __name__ == '__main__':
    unittest.main()
