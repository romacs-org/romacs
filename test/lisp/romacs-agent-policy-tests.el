;;; romacs-agent-policy-tests.el --- Full permissions: tool calls allowed unasked -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude))

(defmacro romacs-agent-policy-tests--with (live &rest body)
  "BODY with `session', registered, its records live when LIVE; `answered'
collects what it answers; timers run at once."
  (declare (indent 1))
  `(let* ((romacs-agent--sessions (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "p" :harness 'claude :caught-up ,live :state 'running))
          (answered nil))
     (puthash "p" session romacs-agent--sessions)
     (cl-letf (((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args)))
               ((symbol-function 'romacs-agent-respond) (lambda (_s id allow &rest _) (push (list id allow) answered))))
       ,@body)))

(defun romacs-agent-policy-tests--ask (session id &optional user)
  (puthash id t (romacs-agent-approvals session))
  (romacs-agent--raise session (list :type 'approval :request-id id :tool "Bash" :user user)))

(ert-deftest romacs-agent-policy-allows-a-tool-call-unasked ()
  (romacs-agent-policy-tests--with t
    (let ((romacs-agent-approval-policy 'allow))
      (romacs-agent-policy-tests--ask session "a1")
      (should (equal answered '(("a1" t))))
      ;; Nothing waits on the user meanwhile.
      (should (eq (romacs-agent-state session) 'running))
      ;; What only the user can answer is theirs, and waits for them.
      (romacs-agent-policy-tests--ask session "q1" t)
      (should (equal answered '(("a1" t))))
      (should (eq (romacs-agent-state session) 'waiting-approval))
      ;; A tool call answered meanwhile leaves the question waiting.
      (romacs-agent--raise session '(:type answered :request-id "a1"))
      (should (eq (romacs-agent-state session) 'waiting-approval))
      ;; Its turn ends, unanswered: moot.
      (romacs-agent--raise session '(:type done :seq 9))
      (should-not (romacs-agent-asks session))
      ;; Two calls at once: each answered once.
      (setq answered nil)
      (romacs-agent-policy-tests--ask session "b1")
      (romacs-agent-policy-tests--ask session "b2")
      (should (equal (sort (mapcar #'car answered) #'string<) '("b1" "b2"))))))

(ert-deftest romacs-agent-policy-ask-asks ()
  (romacs-agent-policy-tests--with t
    (let ((romacs-agent-approval-policy 'ask))
      (romacs-agent-policy-tests--ask session "a1")
      (should-not answered)
      (should (eq (romacs-agent-state session) 'waiting-approval)))))

(ert-deftest romacs-agent-policy-catching-up-allows-what-waits ()
  (romacs-agent-policy-tests--with nil
    (let ((romacs-agent-approval-policy 'allow))
      ;; Met in the replay: nothing written then.
      (romacs-agent-policy-tests--ask session "late")
      (romacs-agent-policy-tests--ask session "question" t)
      (should-not answered)
      ;; Caught up: the tool call still waiting is allowed, the question kept.
      (setf (romacs-agent-caught-up session) t)
      (romacs-agent--allow-tool-calls session)
      (should (equal answered '(("late" t))))
      ;; A copy drawn again, or a policy turned to ask, answers nothing.
      (setq answered nil)
      (let ((copy (romacs-agent--make :id "p" :harness 'claude :caught-up t)))
        (puthash "x" t (romacs-agent-approvals copy))
        (romacs-agent--allow-tool-calls copy))
      (let ((romacs-agent-approval-policy 'ask)) (romacs-agent--allow-tool-calls session))
      (should-not answered)
      ;; An exit takes what waited with it.
      (romacs-agent--raise session '(:type exit :status 0))
      (should-not (romacs-agent-asks session))
      (romacs-agent--allow-tool-calls session)
      (should-not answered))))

(ert-deftest romacs-agent-policy-claude-marks-what-is-the-user-s ()
  (let ((session (romacs-agent--make :id "c" :harness 'claude :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal)))
        (ask (lambda (tool)
               (romacs-mcp--object "type" "control_request" "request_id" tool
                                   "request" (romacs-mcp--object "subtype" "can_use_tool" "tool_name" tool
                                                                 "input" (romacs-mcp--object))))))
    (should-not (plist-get (car (romacs-agent--claude-events session (funcall ask "Bash"))) :user))
    (should (plist-get (car (romacs-agent--claude-events session (funcall ask "ExitPlanMode"))) :user))
    (should (plist-get (car (romacs-agent--claude-events session (funcall ask "AskUserQuestion"))) :user))))

(provide 'romacs-agent-policy-tests)
;;; romacs-agent-policy-tests.el ends here
