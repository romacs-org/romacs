"""The Romacs batch adaptation preserves GNU ERT's execution machinery."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ert_report
import run


class BatchAdapter(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        sys.path.insert(0, str(run.ROOT / 'tools/lib'))
        import gnu_oracle
        self.binary = gnu_oracle.require(os.environ.get('ROMACS_GNU_ORACLE')
                                        or os.environ.get('ROMACS_FORCE_ORACLE_PATH'))['path']
        self.ert = run.ROOT / 'gnu-lisp/emacs-lisp/ert.el'
        self.adapter = run.ROOT / 'rlisp/romacs-ert-batch.el'

    def evaluate(self, source):
        result = subprocess.run([self.binary, '-Q', '--batch', '--eval', source],
                                env=run.gnu_env(self.home), capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout

    def test_execution_context_changes_only_the_precautionary_window_wrapper(self):
        source = r'''
(progn
  (require 'cl-lib)
  (defun adapter-read-definition (file)
    (with-temp-buffer
      (insert-file-contents file)
      (search-forward "(defun ert--run-test-internal ")
      (backward-char (length "(defun ert--run-test-internal "))
      (read (current-buffer))))
  (let ((replacements 0))
    (cl-labels ((adapt (form)
                  (if (atom form) form
                    (cons (if (eq (car form) 'save-window-excursion)
                              (progn (cl-incf replacements) 'romacs-ert--batch-context)
                            (car form))
                          (mapcar #'adapt (cdr form))))))
      (let ((expected (adapt (adapter-read-definition GNU-ERT))))
        (prin1 (list (= replacements 1)
                     (equal expected (adapter-read-definition ADAPTER))))))))
'''.replace('GNU-ERT', json.dumps(str(self.ert))).replace('ADAPTER', json.dumps(str(self.adapter)))
        self.assertEqual(self.evaluate(source), '(t t)')

    def test_adapter_rejects_interactive_use_and_keeps_batch_runner_identity(self):
        source = r'''
(progn
  (load GNU-ERT nil t t)
  (let* ((batch (symbol-function 'ert-run-tests-batch))
         (internal (symbol-function 'ert--run-test-internal))
         (formatter (symbol-function 'ert--pp-with-indentation-and-newline))
         (rejected (let ((noninteractive nil))
                     (condition-case nil (load ADAPTER nil t t) (error 'rejected)))))
    (unless (and (eq rejected 'rejected)
                 (eq internal (symbol-function 'ert--run-test-internal))
                 (eq formatter (symbol-function 'ert--pp-with-indentation-and-newline)))
      (error "Interactive load changed ERT"))
    (load ADAPTER nil t t)
    (defvar adapter-body-ran nil)
    (let ((runtime-guard (let ((noninteractive nil))
                           (condition-case nil
                               (eval '(romacs-ert--batch-context (setq adapter-body-ran t)) t)
                             (error 'rejected)))))
      (prin1 (list (eq batch (symbol-function 'ert-run-tests-batch))
                   (eq runtime-guard 'rejected) (not adapter-body-ran))))))
'''.replace('GNU-ERT', json.dumps(str(self.ert))).replace('ADAPTER', json.dumps(str(self.adapter)))
        self.assertEqual(self.evaluate(source), '(t t t)')

    def test_actual_gnu_assertions_errors_and_temp_buffer_cleanup(self):
        fixture = self.home / 'adapter-tests.el'
        tests = '''
(fset 'current-window-configuration (lambda (&rest _) (error "WINDOW-CAPTURE-MUST-NOT-RUN")))
(fset 'forward-comment (lambda (&rest _) (error "EDITOR-INDENTATION-MUST-NOT-RUN")))
(defvar adapter-cleaned nil)
(defvar adapter-test-buffer nil)
(ert-deftest a-cleanup ()
  (setq adapter-test-buffer (current-buffer))
  (unwind-protect (should-error (car 1) :type 'wrong-type-argument)
    (setq adapter-cleaned t)))
(ert-deftest b-cleanup-observed ()
  (should adapter-cleaned)
  (should-not (buffer-live-p adapter-test-buffer)))
(ert-deftest c-real-failure () (should nil))
'''
        fixture.write_text(tests)
        with self.assertRaises(ert_report.ReportError):
            ert_report.run_native(self.binary, fixture, None, self.ert, [], self.home, 20)
        fixture.write_text(f'(load {json.dumps(str(self.adapter))} nil t t)\n' + tests)
        report = ert_report.run_native(self.binary, fixture, None, self.ert, [], self.home, 20)
        self.assertEqual(report['returncode'], 1)
        self.assertEqual(report['execution'], dict(selected=3, started=3, terminal=3))
        self.assertEqual(report['summary'], dict(passed=2, failed=1, skipped=0, total=3))
        self.assertEqual(report['results']['c-real-failure'], dict(outcome='failed', as_expected=False))
        self.assertIn('(ert-test-failed', report['output'])
        self.assertNotIn('EDITOR-INDENTATION-MUST-NOT-RUN', report['output'])


if __name__ == '__main__':
    unittest.main()
