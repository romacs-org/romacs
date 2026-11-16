;;; romacs-mcp-dev.el --- Checks of a Romacs source tree, as MCP tools  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; romacs_dev_check starts a source tree's own `tools/dev-check' (the merge
;; forecast with the trunk, the generated-file check, the lint of the
;; Lisp the branch touches, run by this Romacs, and the Rust tests the change
;; can reach) as a systemd --user unit in the romacs-build slice, and
;; answers at once with the unit and the file its report goes to.  The
;; tree's tool decides what is checked; this file starts it.  No request
;; stays open for the minutes a check takes, so a restart never waits for
;; one, and the check goes on through a restart.  The agents layer tells
;; the calling task when the unit ends (`romacs-mcp-dev-watch-function');
;; a caller never told reads the report, whose last line is the verdict.
;;
;; romacs_dev_guide answers from this Romacs what it offers for writing
;; Lisp for it (romacs-dev-guide.el), and for a worktree what the module
;; AGENTS.md files over its branch's paths say (tools/lib/agents_md.py).
;;
;; The check has this Romacs's environment.  A session started without the
;; Rust toolchain on its PATH gets the forecast and the generated-file
;; check, and is told the tests did not run.

;;; Code:

(require 'cl-lib)
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

(defvar romacs-mcp-dev-watch-function nil
  "Called with a check's UNIT, its REPORT file and the time it was started,
once it runs: non-nil when the session calling the tool will be told of
its end (the agents layer watches it for that task).")

(defun romacs-mcp-dev--unit (worktree)
  "The unit a check of WORKTREE runs as: one at a time for a tree, whose
test log is one too."
  (format "romacs-dev-check-%s-%s"
          (replace-regexp-in-string "[^A-Za-z0-9_.-]" "-" (file-name-nondirectory (directory-file-name worktree)))
          (substring (secure-hash 'sha1 (directory-file-name worktree)) 0 8)))

(defun romacs-mcp-dev--report (unit)
  "The file UNIT's report goes to, beside the test logs of tools/dev-check."
  (expand-file-name (concat unit ".txt")
                    (expand-file-name "romacs/dev-check" (or (getenv "XDG_CACHE_HOME") "~/.cache"))))

(defun romacs-mcp-dev--environment (environment)
  "ENVIRONMENT, entries as in `process-environment', as systemd takes it:
each name once with its first value, as `getenv' reads it, and only the
entries systemd accepts."
  (let ((seen (make-hash-table :test #'equal)) (kept nil))
    (dolist (entry environment (nreverse kept))
      (when (and (string-match "\\`\\([A-Za-z_][A-Za-z0-9_]*\\)=[^\n]*\\'" entry)
                 (not (gethash (match-string 1 entry) seen)))
        (puthash (match-string 1 entry) t seen)
        (push entry kept)))))

(defun romacs-mcp-dev--unit-argv (unit report command directory environment)
  "systemd-run's arguments that run COMMAND in DIRECTORY as the user unit
UNIT in the romacs-build slice, with ENVIRONMENT, its output in REPORT."
  (append (list "--user" "--quiet" "--collect" "--slice=romacs-build" "--expand-environment=no"
                (concat "--unit=" unit) (concat "--working-directory=" directory)
                (concat "--property=StandardOutput=truncate:" report) "--property=StandardError=inherit")
          (mapcar (lambda (entry) (concat "--setenv=" entry)) environment)
          (cons "--" command)))

(defun romacs-mcp-dev--start (unit report command directory environment)
  "Start COMMAND as UNIT (`romacs-mcp-dev--unit-argv'), or say why not."
  (make-directory (file-name-directory report) t)
  (with-temp-buffer
    (unless (eql 0 (apply #'call-process "systemd-run" nil t nil
                          (romacs-mcp-dev--unit-argv unit report command directory environment)))
      (user-error "The check did not start as %s (one of this tree may be running; its report is %s): %s"
                  unit report (string-trim (buffer-string))))))

(defun romacs-mcp-dev--worktree (arguments)
  "The worktree ARGUMENTS name, as a directory."
  (file-name-as-directory (expand-file-name (gethash "worktree" arguments))))

(defun romacs-mcp-dev--check (arguments)
  "Start the check of the worktree ARGUMENTS name as a unit; answer at once
with the unit, its report file, and whether the caller will be told."
  (romacs-mcp--instance arguments)
  (let* ((worktree (romacs-mcp-dev--worktree arguments))
         (target (gethash "targetDir" arguments))
         (command (romacs-mcp-dev--command worktree arguments))
         (unit (romacs-mcp-dev--unit worktree))
         (report (romacs-mcp-dev--report unit))
         (started (float-time)))
    (romacs-mcp-dev--start unit report command worktree
                           (romacs-mcp-dev--environment
                            (append (and target (list (concat "CARGO_TARGET_DIR=" (expand-file-name target))))
                                    process-environment)))
    (let ((watched (and romacs-mcp-dev-watch-function
                        (funcall romacs-mcp-dev-watch-function unit report started))))
      (romacs-mcp--object
       "unit" unit "report" report "watched" (if watched t :false)
       "next" (format (if watched
                          "End your turn now: when the check ends, its exit (0 ok, 1 fail, 2 incomplete) and the report's last lines come as your next message. If none comes, read %s once `systemctl --user is-active %s' no longer says active: its last line is the verdict."
                        "Read %s once `systemctl --user is-active %s' no longer says active: its last line is the verdict.")
                      report unit)))))

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
   "Check a Romacs worktree before its branch is submitted, cheapest first: the merge forecast with the trunk (each conflicting file as additive, overlap or whole), tracked generated files, romacs-lint on the Lisp it touches (run by this Romacs), then the Rust tests the change can reach (tools/lib/impact.py; they build test targets into targetDir, default this Romacs's CARGO_TARGET_DIR; tests false leaves them out). Starts it as a systemd --user unit and answers at once: unit, report (the file the report goes to: a line a stage and a finding, each with its fix, the last line the verdict), watched (true: your task is sent the unit's exit, 0 ok, 1 fail, 2 incomplete, and the report's last lines when it ends; end your turn). One check of a worktree runs at a time; it goes on through a restart."
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
