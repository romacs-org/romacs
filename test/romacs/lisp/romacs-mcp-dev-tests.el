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

(defmacro romacs-mcp-dev-tests--starting (started &rest body)
  "BODY with each unit start's arguments (UNIT REPORT COMMAND DIRECTORY
ENVIRONMENT) put in STARTED instead of run."
  (declare (indent 1))
  `(let ((,started nil))
     (cl-letf (((symbol-function 'romacs-mcp-dev--start) (lambda (&rest args) (setq ,started args))))
       ,@body)))

(ert-deftest romacs-mcp-dev-tests-a-check-is-a-unit-answered-at-once ()
  "The check starts as a unit of its own tree, in the tree, with the trunk,
the target and the tests switch it was given; the answer names the unit
and the report at once."
  (let ((tree (romacs-mcp-dev-tests--tree 1)) (romacs-mcp-dev-watch-function nil))
    (unwind-protect
        (romacs-mcp-dev-tests--starting started
          (let ((answer (romacs-mcp-dev--check (romacs-mcp--object "worktree" tree "trunk" "main" "tests" :false
                                                                  "targetDir" "/somewhere/target"))))
            (pcase-let ((`(,unit ,report ,command ,directory ,environment) started))
              (should (equal unit (romacs-mcp-dev--unit tree)))
              (should (equal (gethash "unit" answer) unit))
              (should (equal (gethash "report" answer) report))
              (should (eq (gethash "watched" answer) :false))
              (should (string-match-p (regexp-quote report) (gethash "next" answer)))
              (should (equal command (list (expand-file-name "tools/dev-check" tree)
                                           "--binary" (expand-file-name invocation-name invocation-directory)
                                           "--trunk" "main" "--no-tests")))
              (should (equal directory tree))
              (should (equal (seq-filter (lambda (e) (string-prefix-p "CARGO_TARGET_DIR=" e)) environment)
                             '("CARGO_TARGET_DIR=/somewhere/target")))
              (should (member (concat "PATH=" (getenv "PATH")) environment)))))
      (delete-directory tree t))))

(ert-deftest romacs-mcp-dev-tests-a-task-calling-is-told-and-given-the-report ()
  "With the agents layer watching for the caller, the answer says so, and
names the report a caller never told reads."
  (let* ((tree (romacs-mcp-dev-tests--tree 0)) (watched nil)
         (romacs-mcp-dev-watch-function (lambda (unit report started) (setq watched (list unit report (numberp started))))))
    (unwind-protect
        (romacs-mcp-dev-tests--starting _started
          (let ((answer (romacs-mcp-dev--check (romacs-mcp--object "worktree" tree))))
            (should (eq (gethash "watched" answer) t))
            (should (equal watched (list (gethash "unit" answer) (gethash "report" answer) t)))
            (should (string-match-p (regexp-quote (gethash "report" answer)) (gethash "next" answer)))))
      (delete-directory tree t))))

(ert-deftest romacs-mcp-dev-tests-the-environment-systemd-takes ()
  "Each name once, its first value as `getenv' reads it; entries systemd
refuses are left out."
  (should (equal (romacs-mcp-dev--environment '("A=1" "B=2" "A=3" "BASH_FUNC_f%%=() { :; }" "C=x\ny" "UNSET"))
                 '("A=1" "B=2"))))

(ert-deftest romacs-mcp-dev-tests-the-unit-is-collected-in-the-build-slice ()
  (should (equal (romacs-mcp-dev--unit-argv "u" "/r.txt" '("/t/tools/dev-check" "--binary" "/b$x") "/t/" '("A=1"))
                 '("--user" "--quiet" "--collect" "--slice=romacs-build" "--expand-environment=no"
                   "--unit=u" "--working-directory=/t/" "--property=StandardOutput=truncate:/r.txt"
                   "--property=StandardError=inherit" "--setenv=A=1" "--" "/t/tools/dev-check" "--binary" "/b$x"))))

(ert-deftest romacs-mcp-dev-tests-a-unit-that-did-not-start-is-said ()
  "systemd-run runs with the unit's arguments; its refusal (a check of
the tree running) is an error naming the unit and its report."
  (let* ((bin (make-temp-file "romacs-mcp-dev-tests-bin-" t))
         (calls (expand-file-name "calls" bin))
         (report (expand-file-name "reports/u.txt" bin))
         (exec-path (list bin)))
    (unwind-protect
        (cl-flet ((fake (status)
                    (with-temp-file (expand-file-name "systemd-run" bin)
                      (insert (format "#!/bin/sh\nprintf '%%s\\n' \"$@\" > %s\necho 'Unit u.service was already loaded'\nexit %d\n"
                                      calls status)))
                    (set-file-modes (expand-file-name "systemd-run" bin) #o755)))
          (fake 0)
          (romacs-mcp-dev--start "u" report '("/t/tools/dev-check") "/t/" '("A=1"))
          (should (file-directory-p (file-name-directory report)))
          (should (equal (with-temp-buffer (insert-file-contents calls) (split-string (buffer-string) "\n" t))
                         (romacs-mcp-dev--unit-argv "u" report '("/t/tools/dev-check") "/t/" '("A=1"))))
          (fake 1)
          (let ((said (cadr (should-error (romacs-mcp-dev--start "u" report '("/t/tools/dev-check") "/t/" nil)
                                          :type 'user-error))))
            (should (string-match-p (regexp-quote report) said))
            (should (string-match-p "already loaded" said))))
      (delete-directory bin t))))

(ert-deftest romacs-mcp-dev-tests-guide-at-once ()
  "A function by name, and no query, are answered at once."
  (should (string-prefix-p "car (list): primitive" (romacs-mcp-dev--guide (romacs-mcp--object "query" "car"))))
  (should (string-prefix-p "What callers of a function are told" (romacs-mcp-dev--guide (romacs-mcp--object)))))

(defun romacs-mcp-dev-tests--defined-since-boot (thing)
  "Return THING, for an aardwolf: no world was built with this function."
  thing)

(ert-deftest romacs-mcp-dev-tests-guide-search-in-steps ()
  "A search of docstrings is answered over polls of the request, each a
short step, and sees a function defined since this Romacs started."
  (let* ((deferred (romacs-mcp-dev--guide (romacs-mcp--object "query" "aardwolf world" "limit" 3)))
         (polls 0) (result nil))
    (should (eq (car deferred) 'romacs-mcp--deferred))
    (while (not (setq result (funcall (cadr deferred)))) (setq polls (1+ polls)))
    (should (> polls 1))
    (should (string-prefix-p "romacs-mcp-dev-tests--defined-since-boot (function): Return THING, for an aardwolf: no world was built with this function.\n"
                             (car result)))))

(ert-deftest romacs-mcp-dev-tests-guide-module-notes ()
  "Given a worktree, the guide answers with what the tree's own
tools/lib/agents_md.py prints for the branch's paths."
  (let ((tree (romacs-mcp-dev-tests--tree 0)))
    (unwind-protect
        (progn
          (make-directory (expand-file-name "tools/lib" tree))
          (with-temp-file (expand-file-name "tools/lib/agents_md.py" tree)
            (insert "import sys\nprint('== notes for', ' '.join(sys.argv[1:]))\n"))
          (let* ((deferred (romacs-mcp-dev--guide (romacs-mcp--object "worktree" tree "trunk" "main")))
                 (end (+ (float-time) 30)) (result nil))
            (while (and (not (setq result (funcall (cadr deferred)))) (< (float-time) end))
              (accept-process-output nil 0.05))
            (should (equal (car result) (format "== notes for notes --root %s --trunk main" tree))))
          (delete-file (expand-file-name "tools/lib/agents_md.py" tree))
          (should-error (romacs-mcp-dev--guide (romacs-mcp--object "worktree" tree)) :type 'user-error))
      (delete-directory tree t))))

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
