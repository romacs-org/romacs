;;; romacs-agent-watch-tests.el --- A task told when the units it waits on end -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-watch))

(ert-deftest romacs-agent-watch-reads-systemd ()
  (let ((states (romacs-agent-watch--states
                 "Id=check-a.service\nActiveState=active\n\nId=check-b.service\nActiveState=inactive\n\nId=c.service\nActiveState=failed\n")))
    (should (equal (gethash "check-a.service" states) "active"))
    (should (equal (mapcar #'car (romacs-agent-watch--ended
                                  (list (cons "t1" '(:unit "check-a")) (cons "t2" '(:unit "check-b.service"))
                                        (cons "t3" '(:unit "c.service")) (cons "t4" '(:unit "unanswered")))
                                  states))
                   ;; Running and unnamed units are still waited on.
                   '("t2" "t3"))))
  (should (equal (romacs-agent-watch--status "x.service: Main process exited, code=exited, status=1/FAILURE\nx.service: Failed with result 'exit-code'.\n")
                 "exited, status=1/FAILURE (failed: exit-code)"))
  (should (equal (romacs-agent-watch--status "x.service: Deactivated successfully.\n") "exited, status=0/SUCCESS"))
  ;; As the user manager logs them: a success only by what it consumed,
  ;; a stop by its own line, a failure with its exit before the tally.
  (should (equal (romacs-agent-watch--status "Started [systemd-run] check.\nx.service: Consumed 3h 30min CPU time over 3h wall clock time, 10.5G memory peak.\n")
                 "exited, status=0/SUCCESS"))
  (should (equal (romacs-agent-watch--status "Stopping [systemd-run] check...\nStopped [systemd-run] check.\nx.service: Consumed 2min CPU time over 4min wall clock time.\n")
                 "stopped"))
  (should (equal (romacs-agent-watch--status "x.service: Main process exited, code=exited, status=2/INVALIDARGUMENT\nx.service: Failed with result 'exit-code'.\nx.service: Consumed 1s CPU time over 2s wall clock time.\n")
                 "exited, status=2/INVALIDARGUMENT (failed: exit-code)"))
  (should (string-prefix-p "unknown" (romacs-agent-watch--status ""))))

(defmacro romacs-agent-watch-tests--with (&rest body)
  "BODY with the lead `lead' calling and its idle task `child', in a
private directory `root'.  `told' collects (TASK . TEXT) a task is told,
`state' is what systemd says of any unit, `journal' its journal."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-watch-" t))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-watch--watches (make-hash-table :test #'equal))
          (romacs-agent-watch--timer nil) (romacs-agent-watch--asking nil)
          (lead (romacs-agent--make :id "lead" :harness 'claude :state 'idle :dir root))
          (child (romacs-agent--make :id "child" :harness 'claude :state 'idle :title "lane" :cwd root
                                     :dir (expand-file-name "child" root) :options '(:parent "lead")))
          (caller "lead") (state "active")
          (journal "check.service: Main process exited, code=exited, status=0/SUCCESS\n")
          (told nil))
     (make-directory (romacs-agent-dir child))
     (dolist (s (list lead child)) (puthash (romacs-agent-id s) s romacs-agent--sessions))
     (romacs-agent--index child)
     (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () caller))
               ((symbol-function 'romacs-agent-watch--unit-state) (lambda (_unit) state))
               ((symbol-function 'romacs-agent-watch--output)
                (lambda (program args then)
                  (funcall then (if (equal program "journalctl") journal
                                  (mapconcat (lambda (unit) (format "Id=%s\nActiveState=%s\n" unit state))
                                             (cdr (member "--" args)) "\n")))
                  nil))
               ((symbol-function 'romacs-agent-tools--tell) (lambda (task text) (push (cons task text) told) task))
               ((symbol-function 'run-at-time) (lambda (&rest _) (timer-create))))
       (unwind-protect (progn ,@body) (delete-directory root t)))))

(defun romacs-agent-watch-tests--watch (&rest pairs)
  (romacs-agent-watch--watch (apply #'romacs-mcp--object pairs)))

(ert-deftest romacs-agent-watch-tells-the-task-not-the-lead ()
  (romacs-agent-watch-tests--with
    (with-temp-file (expand-file-name "check.log" root)
      (dotimes (i 30) (insert (format "line %d\n" i))))
    (let ((answer (romacs-agent-watch-tests--watch "taskId" "child" "unit" "check" "output" "check.log")))
      (should (equal (gethash "unit" answer) "check.service"))
      (should (string-match-p "End your turn" (gethash "next" answer))))
    ;; The same unit again is one watch.
    (romacs-agent-watch-tests--watch "taskId" "child" "unit" "check.service")
    (should (= (length (gethash "child" romacs-agent-watch--watches)) 1))
    (should romacs-agent-watch--timer)
    ;; Still running: nothing is said.
    (romacs-agent-watch--tick)
    (should-not told)
    (setq state "inactive")
    (romacs-agent-watch--tick)
    (should (= (length told) 1))
    (should (eq (car (car told)) child))
    (let ((text (cdr (car told))))
      (should (string-match-p "Unit check.service ended: exited, status=0/SUCCESS" text))
      (should (string-match-p "line 29\\'" text))
      (should (string-match-p "line 10\n" text))
      (should-not (string-match-p "line 9\n" text)))
    ;; Told once; nothing watched, no timer, no record.
    (romacs-agent-watch--tick)
    (should (= (length told) 1))
    (should-not (gethash "child" romacs-agent-watch--watches))
    (should-not romacs-agent-watch--timer)
    (should-not (file-exists-p (romacs-agent-watch--file child)))))

(ert-deftest romacs-agent-watch-refusals ()
  (romacs-agent-watch-tests--with
    ;; Only the task itself or its parent.
    (setq caller "someone")
    (should-error (romacs-agent-watch-tests--watch "taskId" "child" "unit" "check") :type 'user-error)
    ;; The task itself, by default.
    (setq caller "child")
    (should (equal (gethash "taskId" (romacs-agent-watch-tests--watch "unit" "check")) "child"))
    ;; A unit not running cannot be watched.
    (setq state "inactive")
    (should-error (romacs-agent-watch-tests--watch "unit" "other") :type 'user-error)
    ;; A cancelled task's watches go with it.
    (setf (romacs-agent-cancelled child) t)
    (romacs-agent-watch--tick)
    (should-not told)
    (should-not (gethash "child" romacs-agent-watch--watches))))

(ert-deftest romacs-agent-watch-survives-a-restart ()
  (romacs-agent-watch-tests--with
    (romacs-agent-watch-tests--watch "taskId" "child" "unit" "check" "output" "check.log")
    (clrhash romacs-agent-watch--watches)
    (setq romacs-agent-watch--timer nil)
    (romacs-agent-watch--restore)
    (should (equal (plist-get (car (gethash "child" romacs-agent-watch--watches)) :unit) "check.service"))
    (should (equal (plist-get (car (gethash "child" romacs-agent-watch--watches)) :output)
                   (expand-file-name "check.log" root)))
    (should romacs-agent-watch--timer)))

(ert-deftest romacs-agent-watch-a-file-section ()
  (romacs-agent-watch-tests--with
    (let ((index (expand-file-name "INDEX.md" root)))
      (cl-flet ((index (&rest decide)
                  (with-temp-file index
                    (insert "# Requests\n\n## For the train\n- 111 a  fix  1.0 h\n\n## For the lead to decide\n")
                    (dolist (line decide) (insert line "\n"))
                    (insert "\n## Merged\n- 999 z\n"))))
        (index "- 222 b  conflict  2.0 h")
        (setq caller "lead")
        (let ((answer (romacs-agent-watch-tests--watch "taskId" "lead" "file" index "section" "For the lead to decide")))
          (should (string-match-p "§ For the lead to decide" (aref (gethash "watching" answer) 0))))
        ;; What is there when it starts is known; ages rewritten are not new.
        (index "- 222 b  conflict  2.3 h")
        (romacs-agent-watch--tick)
        (should-not told)
        ;; A request held: the lead is told, that line alone.
        (index "- 222 b  conflict  2.4 h" "- 333 c  decide: waive  0.1 h")
        (romacs-agent-watch--tick)
        (should (= (length told) 1))
        (should (eq (car (car told)) lead))
        (should (string-match-p "New under \"For the lead to decide\" in .*INDEX.md" (cdr (car told))))
        (should (string-match-p "333 c" (cdr (car told))))
        (should-not (string-match-p "222 b\\|111 a\\|999 z" (cdr (car told))))
        (romacs-agent-watch--tick)
        (should (= (length told) 1))
        ;; Kept across a restart, what it has seen with it.
        (clrhash romacs-agent-watch--watches)
        (romacs-agent-watch--restore)
        (romacs-agent-watch--tick)
        (should (= (length told) 1))
        ;; Gone and back is new.
        (index "- 222 b  conflict  2.5 h")
        (romacs-agent-watch--tick)
        (index "- 222 b  conflict  2.6 h" "- 333 c  decide again")
        (romacs-agent-watch--tick)
        (should (= (length told) 2))
        ;; A tell that fails is tried again: what was not told is not seen.
        (index "- 222 b  conflict  2.7 h" "- 333 c  decide again" "- 555 e  held")
        (cl-letf (((symbol-function 'romacs-agent-tools--tell) (lambda (&rest _) (error "harness gone"))))
          (romacs-agent-watch--tick))
        (should (= (length told) 2))
        (romacs-agent-watch--tick)
        (should (= (length told) 3))
        (should (string-match-p "555 e" (cdr (car told))))
        ;; Stopped, its file gone: nothing more.
        (delete-file index)
        (romacs-agent-watch-tests--watch "taskId" "lead" "file" index "section" "For the lead to decide" "stop" t)
        (should-not (gethash "lead" romacs-agent-watch--watches))
        (index "- 444 d  new")
        (romacs-agent-watch--tick)
        (should (= (length told) 3))))))

(provide 'romacs-agent-watch-tests)
;;; romacs-agent-watch-tests.el ends here
