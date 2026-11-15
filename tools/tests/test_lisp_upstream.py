import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import lisp_upstream as up

LOG = '\0'.join([
    'commit aaa\x1f2026-11-01\x1fFix a thing', 'M', 'lisp/files.el', 'M', 'src/fileio.c', 'M', 'etc/NEWS',
    '\ncommit bbb\x1f2026-11-02\x1fNew library', 'A', 'lisp/net/new.el', 'M', 'lisp/ldefs-boot.el',
    '\ncommit ccc\x1f2026-11-03\x1fTouch what we dropped', 'M', 'lisp/play/tetris.el', 'M', 'lisp/ChangeLog.4',
    ''])


class Digest(unittest.TestCase):
    def commits(self):
        return up.classify(up.parse_log(LOG), lambda path: path in {'lisp/files.el'})

    def test_files_are_ours_absent_or_new_and_others_mark_the_commit(self):
        a, b, c = self.commits()
        self.assertEqual([(f['path'], f['ours']) for f in a['files']], [('lisp/files.el', 'ours')])
        self.assertEqual(a['also'], ['etc/NEWS', 'src'])
        self.assertEqual([(f['path'], f['ours']) for f in b['files']], [('lisp/net/new.el', 'new')],
                         'ldefs-boot.el is generated; not a port')
        self.assertEqual(b['also'], [])
        self.assertEqual([f['path'] for f in c['files']], ['lisp/play/tetris.el'], 'ChangeLogs are left out')

    def test_select_by_glob_and_ours_only(self):
        commits = self.commits()
        self.assertEqual([c['commit'] for c in up.select(commits, ours_only=True)], ['aaa', 'bbb'])
        self.assertEqual([c['commit'] for c in up.select(commits, path='lisp/net/*')], ['bbb'])
        self.assertEqual([c['commit'] for c in up.select(commits)], ['aaa', 'bbb', 'ccc'])


if __name__ == '__main__':
    unittest.main()
