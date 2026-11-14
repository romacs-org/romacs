;;; romacs-mcp-dev.el --- Checks of a Romacs source tree, as MCP tools  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; romacs_dev_check runs a source tree's own `tools/dev-check' (the merge
;; forecast with the trunk, the generated-file check, the lint of the
;; Lisp the branch touches, run by this Romacs, and the Rust tests the change
;; can reach)
;; and answers with its report when it ends.  The tree's tool
;; decides what is checked; this file starts it and reads it.  It runs as
;; a subprocess, so the command loop goes on meanwhile; a request that is
;; cancelled leaves the check to finish, and nothing of it behind.
;;
;; romacs_dev_guide answers from this Romacs what it offers for writing
;; Lisp for it (romacs-dev-guide.el), and for a worktree what the module
;; AGENTS.md files over its branch's paths say (tools/lib/agents_md.py).
;;
;; The check's subprocess has this Romacs's environment.  A session started
;; without the Rust toolchain on its PATH gets the forecast and the
;; generated-file check, and is told the tests did not run.

;;; Code:

(require 'romacs-mcp)
(require 'romacs-dev-guide)

(defconst romacs-mcp-dev--step 0.008
  "How many seconds of a guide search one poll of its request takes: what
the command loop waits behind it, twenty times a second.")

(defun romacs-mcp-dev--command (worktree arguments)
  "The command that checks WORKTREE as the tool call's ARGUMENTS ask."
  (let ((tool (expand-file-name "tools/dev-check" worktree))
        (trunk (gethash "trunk" arguments)))
    (unless (file-executable-p tool)
      (user-error "No tools/dev-check under %s: merge the trunk into the branch" worktree))
    ;; The lint's host is this Romacs: it is the machine the branch is held to.
    (append (list tool "--binary" (expand-file-name invocation-name invocation-directory))
            (and trunk (list "--trunk" trunk))
            (and (eq (gethash "tests" arguments) :false) (list "--no-tests")))))

(defun romacs-mcp-dev--result (status report)
  "The tool's answer for a check that exited with STATUS and printed REPORT."
  (romacs-mcp--object "verdict" (pcase status (0 "ok") (1 "fail") (2 "incomplete") (_ "error"))
                      "exit" status
                      "report" report))

(defun romacs-mcp-dev--run (command directory environment answer)
  "Run COMMAND in DIRECTORY with ENVIRONMENT ahead of this Romacs's, and
answer the request when it ends with what ANSWER makes of its exit status
and output.  The sentinel runs once the process has ended and its output
is read: it keeps the answer and drops the buffer, whether or not the
request is still there to poll for it."
  (let ((buffer (generate-new-buffer " *romacs-dev-check*"))
        (result nil))
    (condition-case err
        (let ((default-directory directory)
              (process-environment (append environment process-environment)))
          (make-process :name "romacs-dev-check" :buffer buffer :command command
                        :connection-type 'pipe :noquery t
                        :sentinel (lambda (process _event)
                                    (unless (process-live-p process)
                                      (setq result (list (funcall answer (process-exit-status process)
                                                                  (with-current-buffer buffer (buffer-string)))))
                                      (kill-buffer buffer)))))
      (error (kill-buffer buffer) (signal (car err) (cdr err))))
    (romacs-mcp-defer (lambda () result))))

(defun romacs-mcp-dev--worktree (arguments)
  "The worktree ARGUMENTS name, as a directory."
  (file-name-as-directory (expand-file-name (gethash "worktree" arguments))))

(defun romacs-mcp-dev--check (arguments)
  "Check the worktree ARGUMENTS name; answer when its tools/dev-check ends."
  (romacs-mcp--instance arguments)
  (let ((worktree (romacs-mcp-dev--worktree arguments))
        (target (gethash "targetDir" arguments)))
    (romacs-mcp-dev--run (romacs-mcp-dev--command worktree arguments) worktree
                         (and target (list (concat "CARGO_TARGET_DIR=" (expand-file-name target))))
                         #'romacs-mcp-dev--result)))

(defun romacs-mcp-dev--notes (arguments)
  "The module AGENTS.md files over what the worktree ARGUMENTS name
changes, read by the tree's own tools/lib/agents_md.py."
  (let* ((worktree (romacs-mcp-dev--worktree arguments))
         (tool (expand-file-name "tools/lib/agents_md.py" worktree))
         (trunk (gethash "trunk" arguments)))
    (unless (file-readable-p tool)
      (user-error "No tools/lib/agents_md.py under %s: merge the trunk into the branch" worktree))
    (romacs-mcp-dev--run (append (list "python3" "-B" tool "notes" "--root" worktree) (and trunk (list "--trunk" trunk)))
                         worktree nil
                         (lambda (status output)
                           (if (eql status 0) (string-trim-right output)
                             (format "tools/lib/agents_md.py exited %s:\n%s" status output))))))

(defun romacs-mcp-dev--guide (arguments)
  "What this Romacs offers for the query ARGUMENTS give (`romacs-dev-guide').
With a worktree instead, the module AGENTS.md files over what its
branch changes.  A search of every docstring is taken a step at each poll
of the request, the command loop running between them; a function by
name and the empty query are answered at once."
  (romacs-mcp--instance arguments)
  (let ((query (or (gethash "query" arguments) ""))
        (limit (gethash "limit" arguments)))
    (cond
     ((gethash "worktree" arguments) (romacs-mcp-dev--notes arguments))
     ((not (romacs-dev-guide-search-p query)) (romacs-dev-guide query limit))
     (t
      (let ((search (romacs-dev-guide-search-start query)))
        (romacs-mcp-defer
         (lambda ()
           (setq search (romacs-dev-guide-search-step search romacs-mcp-dev--step))
           (and (romacs-dev-guide-search-done-p search)
                (list (romacs-dev-guide-search-result search limit))))))))))

;;;###autoload
(defun romacs-mcp-enable-dev-tools ()
  "Register the tools that check a Romacs source tree."
  (interactive)
  (romacs-mcp-register-tool
   "romacs_dev_check"
   "Check a Romacs worktree before its branch is submitted, cheapest first: the merge forecast with the trunk (each conflicting file as additive, overlap or whole), tracked generated files, romacs-lint on the Lisp it touches (run by this Romacs), then the Rust tests the change can reach (tools/lib/impact.py; they build test targets into targetDir, default this Romacs's CARGO_TARGET_DIR; tests false leaves them out). Answers when done with verdict (ok, fail, incomplete) and the report: a line a stage and a finding, each with its fix."
   (romacs-mcp--schema '(("instance" . "string") ("worktree" . "string") ("trunk" . "string")
                         ("tests" . "boolean") ("targetDir" . "string"))
                       '("worktree"))
   #'romacs-mcp-dev--check (romacs-mcp--object "readOnlyHint" t))
  (romacs-mcp-register-tool
   "romacs_dev_guide"
   "What this Romacs offers for writing Lisp for it, read from the machine. query a function's name: its arguments, kind, a primitive's effect class (pure, reads, mutates-args, global) and Rust file, its docstring, what callers are told of it. query words: the functions whose name or docstring has them all, names first, at most limit (30). No query: the better paths declared for callers, and the primitives by effect class. worktree instead of query: the module AGENTS.md files (boundary, contracts, tests, traps, decisions) over the paths that worktree's branch changes since trunk (beta)."
   (romacs-mcp--schema '(("instance" . "string") ("query" . "string") ("limit" . "integer")
                         ("worktree" . "string") ("trunk" . "string"))
                       nil)
   #'romacs-mcp-dev--guide (romacs-mcp--object "readOnlyHint" t)))

(provide 'romacs-mcp-dev)
;;; romacs-mcp-dev.el ends here
