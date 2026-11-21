;;; romacs-live-test-tests.el --- Running live Lisp's suites  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A fake tree's tools/test-ert prints what the real runner prints for each
;; module it is given, so the run's plumbing (one runner a module, in
;; order; its lines parsed and shown; the result handed back) is tested
;; without a world or a host.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-live-test)

(defvar romacs-live-ledger)

(defun romacs-live-test-tests--tree (script)
  "A fake Romacs tree whose tools/test-ert is SCRIPT (Python, as the real
runner, which is run by python3: git keeps it not executable)."
  (let* ((tree (file-name-as-directory (make-temp-file "romacs-live-test-" t)))
         (runner (expand-file-name "tools/test-ert" tree)))
    (make-directory (file-name-directory runner) t)
    (with-temp-file runner (insert "import sys\nmodule = sys.argv[sys.argv.index('--module') + 1]\n" script))
    tree))

(defun romacs-live-test-tests--run (tree modules)
  "Run MODULES from TREE; the result, waiting for it."
  (let ((result :none) (romacs-live-test--run nil))
    (romacs-live-test-start tree modules (lambda (r) (setq result r)))
    (with-timeout (30 (error "The run did not end"))
      (while (eq result :none) (accept-process-output nil 0.05)))
    result))

(ert-deftest romacs-live-test-runs-each-module-and-reads-the-runner ()
  (let* ((tree (romacs-live-test-tests--tree
                (concat "if module == 'alpha':\n"
                        "    print('PASS       test/romacs/lisp/alpha-tests.el  3/3 skipped=0 0.1s')\n"
                        "    print('ERT complete: files=1 passed=3')\n"
                        "else:\n"
                        "    print('FAIL       test/romacs/lisp/beta-tests.el  1/2 skipped=0 0.1s')\n"
                        "    print('   FAILED  2/2  beta-breaks (0.01 sec) at x')\n"
                        "    print('   FAILED  beta-breaks')\n"
                        "    print('ERT complete: files=1 failed=1')\n"
                        "    sys.exit(1)\n")))
         (buffer (get-buffer-create "*romacs-live-tests*")))
    (unwind-protect
        (let ((result (romacs-live-test-tests--run tree '("alpha" "beta"))))
          (should (equal (mapcar (lambda (f) (list (plist-get f :file) (plist-get f :status))) (plist-get result :files))
                         '(("test/romacs/lisp/alpha-tests.el" pass) ("test/romacs/lisp/beta-tests.el" fail))))
          (should (equal (plist-get result :failed) '("beta-breaks")))
          (should (string-match-p "1 of 2 suite files failed.*beta-breaks" (romacs-live-test-summary result)))
          (should (equal romacs-live-test-last result))
          (with-current-buffer buffer
            (should (string-match-p "── alpha" (buffer-string)))
            (should (eq (get-text-property (1+ (string-match "FAIL " (buffer-string))) 'face) 'romacs-live-test-fail))))
      (kill-buffer buffer)
      (delete-directory tree t))))

(ert-deftest romacs-live-test-says-when-no-suite-ran ()
  (let ((tree (romacs-live-test-tests--tree "print('test-ert: error: No suites selected', file=sys.stderr)\nsys.exit(2)\n")))
    (unwind-protect
        (let ((result (romacs-live-test-tests--run tree '("none"))))
          (should-not (plist-get result :files))
          (should (equal (romacs-live-test-summary result) "No suite ran for none")))
      (delete-directory tree t))))

(ert-deftest romacs-live-test-takes-the-ledgers-libraries-and-a-real-tree ()
  (let ((romacs-live-ledger '(:entries ((:path "lisp/agents/romacs-agent-ui.el") (:path "/elsewhere/x.el")
                                        (:path "lisp/session/romacs-live.el")))))
    (should (equal (romacs-live-test-modules) '("romacs-agent-ui" "romacs-live"))))
  (should-error (romacs-live-test-start temporary-file-directory '("x") #'ignore) :type 'user-error))

(ert-deftest romacs-live-test-the-tool-answers-with-the-run ()
  (require 'romacs-mcp)
  (let* ((tree (romacs-live-test-tests--tree
                "print('PASS       test/romacs/lisp/alpha-tests.el  3/3 skipped=0 0.1s')\n"))
         (arguments (make-hash-table :test #'equal))
         (romacs-live-test--run nil))
    (unwind-protect
        (progn
          (puthash "worktree" tree arguments)
          (puthash "modules" ["alpha"] arguments)
          (let* ((poll (nth 1 (romacs-live-test-mcp arguments)))
                 (value nil))
            (with-timeout (30 (error "The tool did not answer"))
              (while (not (setq value (funcall poll))) (accept-process-output nil 0.05)))
            (let ((json (json-parse-string (json-serialize (car value)))))
              (should (eq (gethash "passed" json) t))
              (should (equal (gethash "file" (aref (gethash "files" json) 0)) "test/romacs/lisp/alpha-tests.el")))))
      (delete-directory tree t))))

(provide 'romacs-live-test-tests)
;;; romacs-live-test-tests.el ends here
