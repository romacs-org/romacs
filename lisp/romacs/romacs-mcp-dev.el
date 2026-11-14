;;; romacs-mcp-dev.el --- Checks of a Romacs source tree, as MCP tools  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; romacs_dev_check runs a source tree's own `tools/dev-check' (the merge
;; forecast with the trunk, the generated-file check, the structural
;; rules) and answers with its report when it ends.  The tree's tool
;; decides what is checked; this file starts it and reads it.  It runs as
;; a subprocess, so the command loop goes on meanwhile; a request that is
;; cancelled leaves the check to finish, and nothing of it behind.
;;
;; The subprocess has this Romacs's environment.  A session started
;; without the Rust toolchain on its PATH gets the forecast and the
;; generated-file check, and is told the structural rules did not run.

;;; Code:

(require 'romacs-mcp)

(defun romacs-mcp-dev--command (worktree arguments)
  "The command that checks WORKTREE as the tool call's ARGUMENTS ask."
  (let ((tool (expand-file-name "tools/dev-check" worktree))
        (trunk (gethash "trunk" arguments)))
    (unless (file-executable-p tool)
      (user-error "No tools/dev-check under %s: merge the trunk into the branch" worktree))
    (append (list tool)
            (and trunk (list "--trunk" trunk))
            (and (eq (gethash "structural" arguments) :false) (list "--no-structural")))))

(defun romacs-mcp-dev--result (status report)
  "The tool's answer for a check that exited with STATUS and printed REPORT."
  (romacs-mcp--object "verdict" (pcase status (0 "ok") (1 "fail") (2 "incomplete") (_ "error"))
                      "exit" status
                      "report" report))

(defun romacs-mcp-dev--check (arguments)
  "Check the worktree ARGUMENTS name; answer when its tools/dev-check ends."
  (romacs-mcp--instance arguments)
  (let* ((worktree (file-name-as-directory (expand-file-name (gethash "worktree" arguments))))
         (command (romacs-mcp-dev--command worktree arguments))
         (target (gethash "targetDir" arguments))
         (buffer (generate-new-buffer " *romacs-dev-check*"))
         (answer nil))
    ;; The sentinel runs once the process has ended and its output is read:
    ;; it keeps the answer and drops the buffer, whether or not the request
    ;; is still there to poll for it.
    (condition-case err
        (let ((default-directory worktree)
              (process-environment
               (if target
                   (cons (concat "CARGO_TARGET_DIR=" (expand-file-name target)) process-environment)
                 process-environment)))
          (make-process :name "romacs-dev-check" :buffer buffer :command command
                        :connection-type 'pipe :noquery t
                        :sentinel (lambda (process _event)
                                    (unless (process-live-p process)
                                      (setq answer (list (romacs-mcp-dev--result
                                                          (process-exit-status process)
                                                          (with-current-buffer buffer (buffer-string)))))
                                      (kill-buffer buffer)))))
      (error (kill-buffer buffer) (signal (car err) (cdr err))))
    (romacs-mcp-defer (lambda () answer))))

;;;###autoload
(defun romacs-mcp-enable-dev-tools ()
  "Register the tools that check a Romacs source tree."
  (interactive)
  (romacs-mcp-register-tool
   "romacs_dev_check"
   "Check a Romacs worktree before its branch is submitted, cheapest first: the merge forecast with the trunk (each conflicting file as additive, overlap or whole), tracked generated files, then, for a change to Rust, the quick structural rules (they build test targets into targetDir, default this Romacs's CARGO_TARGET_DIR; structural false leaves them out). Answers when done with verdict (ok, fail, incomplete) and the report: a line a stage and a finding, each with its fix."
   (romacs-mcp--schema '(("instance" . "string") ("worktree" . "string") ("trunk" . "string")
                         ("structural" . "boolean") ("targetDir" . "string"))
                       '("worktree"))
   #'romacs-mcp-dev--check (romacs-mcp--object "readOnlyHint" t)))

(provide 'romacs-mcp-dev)
;;; romacs-mcp-dev.el ends here
