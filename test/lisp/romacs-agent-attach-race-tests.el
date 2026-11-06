;;; romacs-agent-attach-race-tests.el --- A run gone before its attachment -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defun romacs-agent-attach-race-tests--closed (session)
  "SESSION's connection, closed as a supervisor leaving closes it."
  (let ((process (make-process :name "closed" :command '("true") :noquery t :sentinel #'ignore)))
    (while (process-live-p process) (accept-process-output process 0.01))
    (process-put process 'romacs-agent session)
    (setf (romacs-agent-connection session) process)
    process))

(defun romacs-agent-attach-race-tests--journal (dir &rest records)
  (let ((coding-system-for-write 'utf-8-unix))
    (write-region (mapconcat (lambda (record) (concat (romacs-agent--json record) "\n")) records "")
                  nil (expand-file-name "journal.jsonl" dir) nil 'silent)))

(defmacro romacs-agent-attach-race-tests--with (&rest body)
  (declare (indent 0))
  `(let* ((dir (make-temp-file "romacs-attach-race-" t))
          (session (romacs-agent--make :id "s" :harness 'codex :provider "codex" :dir dir :state 'starting
                                       :journal-start 0)))
     (unwind-protect (cl-letf (((symbol-function 'romacs-agent--drop-relays) #'ignore)) ,@body)
       (delete-directory dir t))))

(ert-deftest romacs-agent-attach-race-a-finished-run-is-read-from-its-journal ()
  ;; It ended before the attachment was answered: nothing was replayed,
  ;; and the journal says it all.
  (romacs-agent-attach-race-tests--with
    (romacs-agent-attach-race-tests--journal
     dir
     (romacs-mcp--object "seq" 1 "t" 1.0 "out" (romacs-mcp--object "type" "turn.started"))
     (romacs-mcp--object "seq" 2 "t" 1.0
                         "out" (romacs-mcp--object "type" "item.completed"
                                                   "item" (romacs-mcp--object "type" "agent_message" "id" "i" "text" "fast")))
     (romacs-mcp--object "seq" 3 "t" 1.0 "out" (romacs-mcp--object "type" "turn.completed"))
     (romacs-mcp--object "seq" 4 "t" 1.0 "exit" 0))
    (let ((next nil))
      (romacs-agent--enqueue session (list "next" nil nil))
      (cl-letf (((symbol-function 'romacs-agent--turn) (lambda (_session text &rest _) (setq next text))))
        (romacs-agent--sentinel (romacs-agent-attach-race-tests--closed session) "finished\n"))
      (should (eq (romacs-agent-state session) 'idle))
      (should (equal (romacs-agent-reply session) "fast"))
      ;; And what waited for it goes.
      (should (equal next "next")))))

(ert-deftest romacs-agent-attach-race-a-run-still-going-is-detached ()
  ;; No exit in the journal: the supervisor may still run.
  (romacs-agent-attach-race-tests--with
    (romacs-agent-attach-race-tests--journal
     dir (romacs-mcp--object "seq" 1 "t" 1.0 "out" (romacs-mcp--object "type" "turn.started")))
    (romacs-agent--sentinel (romacs-agent-attach-race-tests--closed session) "finished\n")
    (should (eq (romacs-agent-state session) 'detached))))

(ert-deftest romacs-agent-attach-race-an-earlier-run-s-exit-is-not-this-one-s ()
  ;; A restored session reads its whole journal: the last run still goes.
  (romacs-agent-attach-race-tests--with
    (romacs-agent-attach-race-tests--journal
     dir
     (romacs-mcp--object "seq" 1 "t" 1.0 "out" (romacs-mcp--object "type" "turn.completed"))
     (romacs-mcp--object "seq" 2 "t" 1.0 "exit" 0)
     (romacs-mcp--object "seq" 3 "t" 1.0 "note" (romacs-mcp--object "type" "run"))
     (romacs-mcp--object "seq" 4 "t" 1.0 "in" "second")
     (romacs-mcp--object "seq" 5 "t" 1.0 "out" (romacs-mcp--object "type" "turn.started")))
    (romacs-agent--sentinel (romacs-agent-attach-race-tests--closed session) "finished\n")
    (should (eq (romacs-agent-state session) 'detached))))

(provide 'romacs-agent-attach-race-tests)
;;; romacs-agent-attach-race-tests.el ends here
