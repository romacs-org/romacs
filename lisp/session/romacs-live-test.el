;;; romacs-live-test.el --- Run the suites of live-loaded Lisp  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `romacs-live-test' runs the ERT suites of the Lisp a live update loaded
;; (romacs-live.el's ledger), from the source tree they came from, with the
;; tree's own runner, tools/test-ert: each suite file in a process of its
;; own with a private HOME, files in parallel, from a batch world it
;; prepares once and keeps.  So a suite passes here as it passes the merge
;; queue's Lisp check, which runs the same runner on the same suites
;; (`tools/test-ert --module LIBRARY': a library's NAME-tests.el and
;; NAME-*-tests.el, a package's directory).  Nothing runs in this session:
;; its state, files and agents stay as they are.  The runner's lines fill
;; *romacs-live-tests* as they come, its logs kept beside them.

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(defvar romacs-live-ledger)
(defvar romacs-live-last-source)
(declare-function romacs-mcp-defer "romacs-mcp" (poll &optional deadline))
(declare-function romacs-mcp--instance "romacs-mcp" (arguments))

(defcustom romacs-live-test-jobs 4
  "How many suite files `romacs-live-test' runs at once."
  :type 'natnum :group 'romacs-live)

(defface romacs-live-test-pass '((t :inherit success))
  "A suite file whose tests passed.")
(defface romacs-live-test-fail '((t :inherit (bold error)))
  "A suite file or test that failed.")
(defface romacs-live-test-dim '((t :inherit shadow))
  "The runner's other lines.")

(defvar romacs-live-test--run nil
  "The run in progress, a plist, or nil.")

(defvar romacs-live-test-last nil
  "The last run's result, a plist: `:tree', `:modules', `:files' (each
\(:file FILE :status pass|fail|incomplete :tests TEXT)), `:failed' (test
names), `:exits' (each module's runner's exit status), `:logs' and
`:seconds'.")

(defun romacs-live-test--tree-p (dir)
  "Whether DIR is a Romacs source tree with its runner."
  (and dir (file-readable-p (expand-file-name "tools/test-ert" dir))))

(defun romacs-live-test-modules ()
  "The libraries the ledger holds live: what their suites test."
  (delete-dups
   (cl-loop for entry in (plist-get (bound-and-true-p romacs-live-ledger) :entries)
            for path = (plist-get entry :path)
            when (string-prefix-p "lisp/" path)
            collect (file-name-base path))))

(defun romacs-live-test--parse (line result)
  "RESULT, the run's plist, with the runner's LINE taken in."
  (cond ((string-match "\\`\\(PASS\\|FAIL\\|INCOMPLETE\\|TIMEOUT\\) +\\([^ ]+\\) +\\(.*\\)\\'" line)
         (plist-put result :files
                    (append (plist-get result :files)
                            (list (list :file (match-string 2 line) :tests (match-string 3 line)
                                        :status (pcase (match-string 1 line)
                                                  ("PASS" 'pass) ("FAIL" 'fail) (_ 'incomplete)))))))
        ((string-match "\\` +FAILED +\\(?:[0-9]+/[0-9]+ +\\)?\\([^ ]+\\)" line)
         (plist-put result :failed (delete-dups (append (plist-get result :failed) (list (match-string 1 line))))))
        (t result)))

(defun romacs-live-test--face (line)
  (cond ((string-match-p "\\`PASS" line) 'romacs-live-test-pass)
        ((string-match-p "\\`\\(?:FAIL\\|INCOMPLETE\\|TIMEOUT\\)\\| FAILED " line) 'romacs-live-test-fail)
        (t 'romacs-live-test-dim)))

(defun romacs-live-test--show (line)
  "Put the runner's LINE at the end of *romacs-live-tests*."
  (when-let* ((buffer (get-buffer "*romacs-live-tests*")))
    (with-current-buffer buffer
      (let ((inhibit-read-only t))
        (save-excursion
          (goto-char (point-max))
          (insert (propertize line 'face (romacs-live-test--face line)) "\n"))))))

(defun romacs-live-test--command (tree module logs)
  "The runner of TREE on MODULE's suites, its logs in LOGS, run by this
session's executable."
  (list "python3" "-B" (expand-file-name "tools/test-ert" tree)
        "--binary" (expand-file-name invocation-name invocation-directory)
        "--module" module "--jobs" (number-to-string romacs-live-test-jobs) "--logs" logs))

(defun romacs-live-test--next (run)
  "Run RUN's next module, or finish RUN when none is left."
  (let ((module (pop (plist-get run :pending))))
    (if (null module)
        (romacs-live-test--finish run)
      (let* ((tree (plist-get run :tree))
             (logs (expand-file-name module (plist-get run :logs)))
             (default-directory (file-name-as-directory tree))
             ;; The tree's runtime and Lisp with this executable, and no
             ;; selection of the caller's environment (TEST_FILES, SELECTOR
             ;; would override the module's suites).
             (process-environment (append (list (concat "ROMACS_RUNTIME_ROOT=" (directory-file-name tree))
                                                (concat "ROMACS_LISP_DIR=" (expand-file-name "lisp" tree)))
                                          (cl-remove-if (lambda (e) (string-match-p "\\`\\(?:ROMACS_RUNTIME_ROOT\\|ROMACS_LISP_DIR\\|TEST_FILES\\|SELECTOR\\)=" e))
                                                        process-environment)))
             (pending ""))
        (romacs-live-test--show (format "── %s" module))
        (condition-case err
            (romacs-live-test--launch run module tree logs pending)
          (error (romacs-live-test--show (format "%s: the runner did not start: %s" module (error-message-string err)))
                 (push (list module 'unstarted 0 0) (plist-get run :exits))
                 (romacs-live-test--next run)))))))

(defun romacs-live-test--launch (run module tree logs pending)
  "Start RUN's runner on MODULE from TREE, its logs in LOGS; PENDING is
the output not yet a whole line.  Its end records the exit and how many
files it reported, and how many of them did not pass."
  (let ((seen (length (plist-get (plist-get run :result) :files))))
    (make-process
     :name (format "romacs-live-test-%s" module) :noquery t :connection-type 'pipe
     :command (romacs-live-test--command tree module logs)
     :filter (lambda (_process text)
               (setq pending (concat pending text))
               (while (string-match "\n" pending)
                 (let ((line (substring pending 0 (match-beginning 0))))
                   (setq pending (substring pending (match-end 0)))
                   (setf (plist-get run :result) (romacs-live-test--parse line (plist-get run :result)))
                   (romacs-live-test--show line))))
     :sentinel (lambda (process _event)
                 (unless (process-live-p process)
                   (let ((files (nthcdr seen (plist-get (plist-get run :result) :files))))
                     (push (list module (process-exit-status process) (length files)
                                 (cl-count-if-not (lambda (f) (eq (plist-get f :status) 'pass)) files))
                           (plist-get run :exits)))
                   (romacs-live-test--next run))))))

(defun romacs-live-test--finish (run)
  "End RUN: its result, kept and handed to its callback."
  (let ((result (append (list :tree (plist-get run :tree) :modules (plist-get run :modules)
                              :logs (plist-get run :logs) :exits (reverse (plist-get run :exits))
                              :seconds (round (- (float-time) (plist-get run :start))))
                        (plist-get run :result))))
    (setq romacs-live-test--run nil romacs-live-test-last result)
    (romacs-live-test--show (romacs-live-test-summary result))
    (when-let* ((callback (plist-get run :callback))) (funcall callback result))))

(defun romacs-live-test--broken (result)
  "The modules whose runner failed in RESULT, by its exit and the files it
reported: one that did not start, exited 0 or 1 at odds with its files
\(1 with none failing: suites all skipped), exited 2 after running files
\(2 with none: it has no suites), or exited otherwise."
  (cl-loop for (module exit files failing) in (plist-get result :exits)
           unless (pcase exit
                    (0 (zerop failing))
                    (1 (> failing 0))
                    (2 (zerop files)))
           collect module))

(defun romacs-live-test-passed-p (result)
  "Whether RESULT's run passed: suite files ran, each passed, and every
module's runner ended well."
  (and (plist-get result :files)
       (cl-every (lambda (f) (eq (plist-get f :status) 'pass)) (plist-get result :files))
       (null (romacs-live-test--broken result))))

(defun romacs-live-test-summary (result)
  "One line saying how RESULT's run went."
  (let* ((files (plist-get result :files))
         (failing (cl-count-if-not (lambda (f) (eq (plist-get f :status) 'pass)) files))
         (broken (romacs-live-test--broken result)))
    (cond (broken (format "The runner failed for %s%s" (string-join broken ", ")
                          (if (plist-get result :failed) (concat "; failed: " (string-join (plist-get result :failed) ", ")) "")))
          ((null files) (format "No suite ran for %s" (string-join (plist-get result :modules) ", ")))
          ((zerop failing) (format "%d suite files passed in %d s" (length files) (plist-get result :seconds)))
          (t (format "%d of %d suite files failed in %d s: %s" failing (length files) (plist-get result :seconds)
                     (string-join (plist-get result :failed) ", "))))))

(defun romacs-live-test-start (tree modules callback)
  "Run MODULES' suites from TREE, a Romacs source tree, calling CALLBACK
with the result (`romacs-live-test-last')."
  (unless (romacs-live-test--tree-p tree)
    (user-error "Not a Romacs source tree with its runner: %s" tree))
  (unless modules (user-error "No library to test: update from a source first, or name one"))
  (unless (cl-every (lambda (m) (and (stringp m) (string-match-p "\\`[[:alnum:]_.+-]+\\'" m))) modules)
    (user-error "Library names are strings like romacs-live: %S" modules))
  (when romacs-live-test--run (user-error "A test run is already going"))
  (with-current-buffer (get-buffer-create "*romacs-live-tests*")
    (romacs-live-test-mode)
    (let ((inhibit-read-only t)) (erase-buffer)))
  (setq romacs-live-test--run
        (list :tree (expand-file-name tree) :modules modules :pending (copy-sequence modules)
              :logs (make-temp-file "romacs-live-tests-" t) :start (float-time) :callback callback
              :result nil :exits nil))
  (romacs-live-test--next romacs-live-test--run))

(defun romacs-live-test--default-tree ()
  "The tree the last update came from, when it was a worktree."
  (let ((source (bound-and-true-p romacs-live-last-source)))
    (and (eq (plist-get source :kind) 'worktree) (plist-get source :directory))))

(define-derived-mode romacs-live-test-mode special-mode "Live tests"
  "The runner's report of a `romacs-live-test' run.")

;;;###autoload
(defun romacs-live-test (tree modules)
  "Run the ERT suites of the libraries the session runs live, from TREE.
TREE is the source tree they came from (the last update's worktree by
default); MODULES the libraries, by default the ledger's.  With a prefix
argument, ask for both.  Each suite file runs in a process of its own
with a private HOME, as the merge queue runs it; nothing runs in this
session.  The lines fill *romacs-live-tests* as they come."
  (interactive
   (let* ((default (romacs-live-test--default-tree))
          (tree (if (or current-prefix-arg (not default))
                    (read-directory-name "Source tree: " default nil t)
                  default))
          (modules (romacs-live-test-modules)))
     (list tree (if (or current-prefix-arg (not modules))
                    (split-string (read-string (format-prompt "Libraries" (string-join modules " "))
                                               nil nil (string-join modules " ")))
                  modules))))
  (romacs-live-test-start tree modules (lambda (result) (message "%s" (romacs-live-test-summary result))))
  (display-buffer "*romacs-live-tests*"))

;;;###autoload
(defun romacs-live-test-mcp (arguments)
  "The `romacs_live_test' tool: ARGUMENTS name the tree and libraries."
  (romacs-mcp--instance arguments)
  (let* ((tree (or (gethash "worktree" arguments) (romacs-live-test--default-tree)
                   (user-error "Give worktree: the source tree the suites come from")))
         (modules (or (append (gethash "modules" arguments) nil) (romacs-live-test-modules)))
         (done nil))
    (romacs-live-test-start
     tree modules
     (lambda (result)
       (setq done (list (let ((table (make-hash-table :test #'equal)))
                          (puthash "summary" (romacs-live-test-summary result) table)
                          (puthash "passed" (if (romacs-live-test-passed-p result) t :false) table)
                          (puthash "files" (vconcat (mapcar (lambda (f) (let ((row (make-hash-table :test #'equal)))
                                                                         (puthash "file" (plist-get f :file) row)
                                                                         (puthash "status" (symbol-name (plist-get f :status)) row)
                                                                         (puthash "tests" (plist-get f :tests) row)
                                                                         row))
                                                            (plist-get result :files)))
                                   table)
                          (puthash "failed" (vconcat (plist-get result :failed)) table)
                          (puthash "logs" (plist-get result :logs) table)
                          table)))))
    (romacs-mcp-defer (lambda () done))))

(provide 'romacs-live-test)
;;; romacs-live-test.el ends here
