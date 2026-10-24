#!/usr/bin/env python3
"""P0 startup-image and untrusted-row harness regressions using real GNU Lisp."""
from pathlib import Path
import runpy
import tempfile
import unittest

MODULE = runpy.run_path(str(Path(__file__).resolve().parent.parent / 'scripts/core-bench'))


class PropsHarness(unittest.TestCase):
    def test_source_boot_and_truthy_non_dump_stats_are_refused(self):
        for value in ('nil', "'((dumped-with-pdumper . nil))"):
            with self.subTest(stats=value), tempfile.TemporaryDirectory() as scratch:
                engine = MODULE['Engine']('romacs', '/usr/bin/emacs', scratch)
                run = engine.run
                engine.run = lambda *args: run('--eval', f'(fset (quote pdumper-stats) (lambda () {value}))', *args)
                with self.assertRaisesRegex(ValueError, 'startup image verification failed'):
                    engine.compile()

    def test_fontlock_elisp_is_explicitly_untrusted(self):
        self.assertIn('buf-prop-fontlock-elisp', MODULE['UNTRUSTED'])


if __name__ == '__main__':
    unittest.main()
