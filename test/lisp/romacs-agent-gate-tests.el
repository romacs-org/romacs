;;; romacs-agent-gate-tests.el --- An edit refused before it lands -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-claude))

(defmacro romacs-agent-gate-tests--with-file (text &rest body)
  "BODY with `file', an Emacs Lisp file holding TEXT."
  (declare (indent 1))
  `(let ((file (make-temp-file "romacs-gate-" nil ".el" ,text)))
     (unwind-protect (progn ,@body) (delete-file file))))

(defun romacs-agent-gate-tests--input (&rest pairs)
  (apply #'romacs-mcp--object pairs))

(ert-deftest romacs-agent-gate-refuses-an-edit-that-breaks-the-file ()
  (romacs-agent-gate-tests--with-file "(defun a ()\n  (list 1 2))\n\n(defun b () 3)\n"
    ;; A dropped paren: refused, saying where; the file as it was.
    (let ((why (romacs-agent-gate-check "Edit" (romacs-agent-gate-tests--input
                                               "file_path" file "old_string" "(list 1 2))" "new_string" "(list 1 2)"))))
      (should (string-match-p "unreadable as Emacs Lisp: the form that starts at line [0-9]+" why))
      (should (string-match-p "(defun a" (with-temp-buffer (insert-file-contents file) (buffer-string)))))
    ;; A sound edit, a Write, and MultiEdit's last step decide.
    (should-not (romacs-agent-gate-check "Edit" (romacs-agent-gate-tests--input
                                                 "file_path" file "old_string" "(list 1 2)" "new_string" "(list 1 2 3)")))
    (should (romacs-agent-gate-check "Write" (romacs-agent-gate-tests--input "file_path" file "content" "(a (b)")))
    (should (romacs-agent-gate-check
             "MultiEdit" (romacs-agent-gate-tests--input
                          "file_path" file
                          "edits" (vector (romacs-agent-gate-tests--input "old_string" "3)" "new_string" "4)")
                                          (romacs-agent-gate-tests--input "old_string" "4)" "new_string" "4))")))))
    ;; Not its to judge: old text not found, a file that never read, another language.
    (should-not (romacs-agent-gate-check "Edit" (romacs-agent-gate-tests--input
                                                 "file_path" file "old_string" "nowhere" "new_string" "(")))
    (should-not (romacs-agent-gate-check "Edit" (romacs-agent-gate-tests--input
                                                 "file_path" "/tmp/x.py" "old_string" "a" "new_string" "(")))
    (let ((romacs-agent-gate nil))
      (should-not (romacs-agent-gate-check "Write" (romacs-agent-gate-tests--input "file_path" file "content" "("))))))

(ert-deftest romacs-agent-gate-a-broken-file-may-be-fixed ()
  (romacs-agent-gate-tests--with-file "(defun a ()\n  (list 1 2)\n"
    (should-not (romacs-agent-gate-check "Edit" (romacs-agent-gate-tests--input
                                                 "file_path" file "old_string" "(list 1 2)" "new_string" "(list 1)")))))

(ert-deftest romacs-agent-gate-claude-s-hook ()
  (romacs-agent-gate-tests--with-file "(a)\n"
    (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
           (session (romacs-agent--make :id "g" :harness 'claude :caught-up t :tools (make-hash-table :test #'equal)
                                        :streamed (make-hash-table :test #'equal)))
           (callback (lambda (id old new)
                       (romacs-mcp--object
                        "type" "control_request" "request_id" id
                        "request" (romacs-mcp--object
                                   "subtype" "hook_callback" "callback_id" "romacs-gate"
                                   "input" (romacs-mcp--object "hook_event_name" "PreToolUse" "tool_name" "Edit"
                                                               "tool_input" (romacs-mcp--object "file_path" file
                                                                                                "old_string" old
                                                                                                "new_string" new)))))))
      (puthash "g" session romacs-agent--sessions)
      ;; The run starts: initialize, with Romacs's hooks; nothing is ready yet.
      (let ((init (romacs-agent--claude-next session)))
        (should (equal (gethash "subtype" (gethash "request" init)) "initialize"))
        (should (gethash "PreToolUse" (gethash "hooks" (gethash "request" init))))
        (dolist (event (romacs-agent--claude-input-events session init))
          (romacs-agent--raise session event))
        (should-not (romacs-agent--claude-next session))
        (dolist (event (romacs-agent--claude-events
                        session (romacs-mcp--object "type" "control_response"
                                                    "response" (romacs-mcp--object "subtype" "success"
                                                                                   "request_id" (gethash "request_id" init)))))
          (romacs-agent--raise session event))
        (should (eq (romacs-agent--claude-next session) :ready)))
      ;; A breaking edit: owed, judged as its reply is written, the
      ;; refusal shown from the written reply.
      (let ((events (romacs-agent--claude-events session (funcall callback "h1" "(a)" "(a"))))
        (should-not (cl-find 'status events :key (lambda (e) (plist-get e :type))))
        (dolist (event events) (romacs-agent--raise session event)))
      (let* ((reply (romacs-agent--claude-next session))
             (answer (gethash "hookSpecificOutput" (gethash "response" (gethash "response" reply)))))
        (should (equal (gethash "request_id" (gethash "response" reply)) "h1"))
        (should (equal (gethash "permissionDecision" answer) "deny"))
        ;; Its echo is known by its request id, not taken for another line's.
        (should (equal (romacs-agent--step-id reply) "h1"))
        (let ((paid (romacs-agent--claude-input-events session reply)))
          (should (string-match-p "refused" (plist-get (cl-find 'status paid :key (lambda (e) (plist-get e :type))) :text)))
          (should-not (cl-find 'answered paid :key (lambda (e) (plist-get e :type))))
          (dolist (event paid) (romacs-agent--raise session event))))
      (should (eq (romacs-agent--claude-next session) :ready))
      ;; A sound edit: nothing to say.
      (dolist (event (romacs-agent--claude-events session (funcall callback "h2" "(a)" "(b)")))
        (romacs-agent--raise session event))
      (should (equal (hash-table-count (gethash "response" (gethash "response" (romacs-agent--claude-next session)))) 0)))))

(ert-deftest romacs-agent-gate-a-hook-met-in-a-replay-is-judged-when-written ()
  ;; Asked while no editor was attached: replayed, it is owed, and judged
  ;; once written, not answered with nothing.
  (romacs-agent-gate-tests--with-file "(a)\n"
    (let ((session (romacs-agent--make :id "r" :harness 'claude :tools (make-hash-table :test #'equal)
                                       :streamed (make-hash-table :test #'equal)
                                       :driver-state (list :initialized t :ready t))))
      (dolist (event (romacs-agent--claude-events
                      session (romacs-mcp--object
                               "type" "control_request" "request_id" "late"
                               "request" (romacs-mcp--object
                                          "subtype" "hook_callback"
                                          "input" (romacs-mcp--object "hook_event_name" "PreToolUse" "tool_name" "Write"
                                                                      "tool_input" (romacs-mcp--object "file_path" file
                                                                                                       "content" "("))))))
        (romacs-agent--raise session event))
      (should (equal (gethash "permissionDecision"
                              (gethash "hookSpecificOutput"
                                       (gethash "response" (gethash "response" (romacs-agent--claude-next session)))))
                     "deny")))))

(ert-deftest romacs-agent-gate-a-run-of-before-is-ready-as-it-is ()
  ;; A run that spoke before any initialize (started by older code) is not
  ;; initialized in the middle of its conversation.
  (let ((session (romacs-agent--make :id "o" :harness 'claude :driver-state (list :run 1))))
    (dolist (event (romacs-agent--claude-input-events
                    session (romacs-mcp--object "type" "user" "message" (romacs-mcp--object "role" "user" "content" "hi"))))
      (romacs-agent--raise session event))
    (should (eq (romacs-agent--claude-next session) :ready))
    ;; A new run takes the driver as defined now.
    (setf (romacs-agent-driver session) '(:stale t))
    (romacs-agent--raise session '(:type run :seq 9))
    (should-not (plist-get (romacs-agent--driver session) :stale))))

(provide 'romacs-agent-gate-tests)
;;; romacs-agent-gate-tests.el ends here
