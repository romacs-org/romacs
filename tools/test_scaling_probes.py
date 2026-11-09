#!/usr/bin/env python3
"""tools/scaling_probes.py: the reader finds each probe past strings,
comments and character literals; the split loses nothing; and a branch that
still adds a probe to the old probes.el merges it into its own file."""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import scaling_probes  # noqa: E402

OLD = """;;; probes.el --- probes  -*- lexical-binding: t -*-

(defmacro scaling-probe (name &rest plist) `(push ',name scaling-probes))

(defun scaling--helper () "a ) string ( with parens" ?\\( ?\\))

;; Grows a list.
;; Two lines of comment.
(scaling-probe lists :n 10
  :setup (list "(" ?\\" ";")
  :body (length it))

(scaling-probe vectors :n 20 :body (vector ?\\) ?a))  ; trailing

(scaling--completion-probe completion-basic basic)

(provide 'scaling-probes)
"""


class SplitTest(unittest.TestCase):
    def test_each_probe_moves_with_its_comment_and_nothing_is_lost(self):
        harness, probes = scaling_probes.split_text(OLD)
        self.assertEqual(sorted(probes), ['completion-basic', 'lists', 'vectors'])
        self.assertIn(';; Grows a list.\n;; Two lines of comment.\n(scaling-probe lists', probes['lists'])
        self.assertIn('(defun scaling--helper ()', harness)
        self.assertNotIn('scaling-probe lists', harness)
        self.assertLess(harness.index('(load file nil t)'), harness.index("(provide 'scaling-probes)"))
        text = harness + ''.join(probes.values())
        for line in OLD.split('\n'):
            self.assertIn(line, text)
        self.assertEqual(scaling_probes.split_text(harness)[0], harness, 'splitting the harness again is a no-op')


def git(root, *args, check=True):
    return subprocess.run(['git', '-C', str(root), *args], check=check, capture_output=True, text=True)


class MergeTest(unittest.TestCase):
    def test_a_branch_probe_lands_in_its_own_file(self):
        with tempfile.TemporaryDirectory(prefix='probes-merge-', dir=os.environ.get('TMPDIR')) as tmp:
            root = Path(tmp)
            git(root, 'init', '-q', '-b', 'trunk')
            for key, value in (('user.name', 'Fixture'), ('user.email', 'fixture@example.invalid'),
                               ('commit.gpgsign', 'false')):
                git(root, 'config', key, value)
            path = root / scaling_probes.DIR / scaling_probes.FILE
            path.parent.mkdir(parents=True)
            path.write_text(OLD)
            git(root, 'add', '-A')
            git(root, 'commit', '-qm', 'base')
            git(root, 'checkout', '-q', '-b', 'lane')
            path.write_text(OLD.replace("(provide 'scaling-probes)",
                                        "(scaling-probe strings :n 30 :body (concat it))\n\n(provide 'scaling-probes)"))
            git(root, 'commit', '-qam', 'lane adds a probe')
            git(root, 'checkout', '-q', 'trunk')
            for name, text in scaling_probes.split({(scaling_probes.DIR / scaling_probes.FILE).as_posix(): OLD}).items():
                (root / scaling_probes.DIR / name).parent.mkdir(parents=True, exist_ok=True)
                (root / scaling_probes.DIR / name).write_text(text)
            git(root, 'add', '-A')
            git(root, 'commit', '-qm', 'split')
            git(root, 'checkout', '-q', 'lane')
            git(root, 'merge', '--no-commit', '-X', 'no-renames', 'trunk', check=False)
            done = subprocess.run([sys.executable, str(Path(scaling_probes.__file__)), 'merge', '--root', str(root)],
                                  capture_output=True, text=True)
            self.assertEqual(done.returncode, 0, done.stderr)
            probes = root / scaling_probes.DIR / 'probes'
            self.assertEqual(sorted(p.stem for p in probes.glob('*.el')),
                             ['completion-basic', 'lists', 'strings', 'vectors'])
            self.assertNotIn('(scaling-probe ', path.read_text())
            self.assertEqual(path.read_text(), (root / scaling_probes.DIR / scaling_probes.FILE).read_text())
            git(root, 'commit', '-qm', 'merge trunk')
            self.assertFalse(git(root, 'status', '--porcelain').stdout)


if __name__ == '__main__':
    unittest.main()
