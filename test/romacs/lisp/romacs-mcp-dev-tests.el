;;; romacs-mcp-dev-tests.el --- romacs_dev_check runs a tree's own check  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The tool against a tree whose tools/dev-check is a script that says how
;; it was called: the arguments reach it, and its exit is the verdict.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'seq)
(require 'romacs-mcp-dev)

(defun romacs-mcp-dev-tests--tree (status)
  "A new directory whose tools/dev-check prints its call and exits with STATUS."
  (let ((tree (file-name-as-directory (make-temp-file "romacs-mcp-dev-tests-" t))))
    (make-directory (expand-file-name "tools" tree))
    (with-temp-file (expand-file-name "tools/dev-check" tree)
      (insert (format "#!/bin/sh\necho \"args:$*\"\necho \"target:$CARGO_TARGET_DIR\"\necho \"in:$(pwd -P)\"\nexit %d\n" status)))
    (set-file-modes (expand-file-name "tools/dev-check" tree) #o755)
    tree))

(defun romacs-mcp-dev-tests--call (&rest fields)
  "The tool's answer to a call with FIELDS, once its check has ended."
  (let ((deferred (romacs-mcp-dev--check (apply #'romacs-mcp--object fields)))
        (end (+ (float-time) 30)) (result nil))
    (should (eq (car deferred) 'romacs-mcp--deferred))
    (while (and (not (setq result (funcall (cadr deferred)))) (< (float-time) end))
      (accept-process-output nil 0.05))
    (car result)))

(ert-deftest romacs-mcp-dev-tests-verdict-and-report ()
  "The check's exit is the verdict and its output the report; it runs in
the tree, with the trunk, the target and the structural switch it was given."
  (let ((tree (romacs-mcp-dev-tests--tree 1)))
    (unwind-protect
        (let ((answer (romacs-mcp-dev-tests--call "worktree" tree "trunk" "main" "structural" :false
                                                  "targetDir" "/somewhere/target")))
          (should (equal (gethash "verdict" answer) "fail"))
          (should (equal (gethash "exit" answer) 1))
          (should (equal (gethash "report" answer)
                         (format "args:--trunk main --no-structural\ntarget:/somewhere/target\nin:%s\n"
                                 (directory-file-name (file-truename tree))))))
      (delete-directory tree t))))

(ert-deftest romacs-mcp-dev-tests-defaults ()
  "A call that names only the tree runs every stage against the default trunk."
  (let ((tree (romacs-mcp-dev-tests--tree 0)))
    (unwind-protect
        (let ((answer (romacs-mcp-dev-tests--call "worktree" tree)))
          (should (equal (gethash "verdict" answer) "ok"))
          (should (string-prefix-p "args:\n" (gethash "report" answer))))
      (delete-directory tree t))))

(ert-deftest romacs-mcp-dev-tests-incomplete ()
  "A check that could not run a stage is not a pass."
  (let ((tree (romacs-mcp-dev-tests--tree 2)))
    (unwind-protect
        (should (equal (gethash "verdict" (romacs-mcp-dev-tests--call "worktree" tree)) "incomplete"))
      (delete-directory tree t))))

(ert-deftest romacs-mcp-dev-tests-nothing-left-behind ()
  "A check no one polls for still ends and leaves no buffer; neither does
one that could not start."
  (cl-flet ((buffers () (seq-count (lambda (buffer) (string-prefix-p " *romacs-dev-check*" (buffer-name buffer)))
                                   (buffer-list))))
    (let ((tree (romacs-mcp-dev-tests--tree 0)) (before (buffers)) (end (+ (float-time) 30)))
      (unwind-protect
          (progn
            (romacs-mcp-dev--check (romacs-mcp--object "worktree" tree))
            (while (and (> (buffers) before) (< (float-time) end)) (accept-process-output nil 0.05))
            (should (= (buffers) before))
            (let ((exec-path nil))
              (with-temp-file (expand-file-name "tools/dev-check" tree) (insert "#!/nonexistent/interpreter\n"))
              (should-error (romacs-mcp-dev--check (romacs-mcp--object "worktree" tree)))
              (should (= (buffers) before))))
        (delete-directory tree t)))))

(ert-deftest romacs-mcp-dev-tests-no-tool ()
  "A tree without the check is refused, with what to do."
  (let ((tree (file-name-as-directory (make-temp-file "romacs-mcp-dev-tests-" t))))
    (unwind-protect
        (should (string-match-p "merge the trunk into the branch"
                                (cadr (should-error (romacs-mcp-dev--check (romacs-mcp--object "worktree" tree))
                                                    :type 'user-error))))
      (delete-directory tree t))))

(provide 'romacs-mcp-dev-tests)
;;; romacs-mcp-dev-tests.el ends here
