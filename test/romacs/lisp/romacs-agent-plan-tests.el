;;; romacs-agent-plan-tests.el --- Plan mode, as T3 Code has it -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex)
  (require 'romacs-agent-ui))

(defmacro romacs-agent-plan-tests--with (harness &rest body)
  "BODY with `session' of HARNESS, live; `written' and `answered' collect."
  (declare (indent 1))
  `(let* ((romacs-agent--sessions (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "p" :harness ,harness :provider (symbol-name ,harness) :state 'idle
                                       :caught-up t :permission-mode "bypassPermissions"
                                       :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal)))
          (written nil) (answered nil))
     (puthash "p" session romacs-agent--sessions)
     (cl-letf (((symbol-function 'romacs-agent--manifest-put) #'ignore)
               ((symbol-function 'process-live-p) (lambda (_) t))
               ((symbol-function 'romacs-agent--line) (lambda (_s object &optional _j) (push object written)))
               ((symbol-function 'romacs-agent--note) #'ignore)
               ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args)))
               ((symbol-function 'romacs-agent-respond)
                (lambda (_s id allow &optional message) (push (list id allow message) answered))))
       ,@body)))

(ert-deftest romacs-agent-plan-claude-switches-live-and-back ()
  (romacs-agent-plan-tests--with 'claude
    (romacs-agent-set-plan session t)
    (should (plist-get (romacs-agent-options session) :plan))
    (should (equal (plist-get (romacs-agent-options session) :plan-from) "bypassPermissions"))
    (let ((request (gethash "request" (car written))))
      (should (equal (gethash "subtype" request) "set_permission_mode"))
      (should (equal (gethash "mode" request) "plan")))
    ;; Back to the mode it had.
    (romacs-agent-set-plan session nil)
    (should-not (plist-get (romacs-agent-options session) :plan))
    (should (equal (gethash "mode" (gethash "request" (car written))) "bypassPermissions"))
    ;; Its status says the mode.
    (should (equal (car (romacs-agent--claude-events
                         session (romacs-mcp--object "type" "system" "subtype" "status" "status" :null
                                                     "permissionMode" "plan")))
                   '(:type mode :permission-mode "plan")))))

(ert-deftest romacs-agent-plan-the-harness-is-put-in-plan-mode-when-ready ()
  ;; However it started (a launch, a resume, an attach): once ready, put
  ;; in plan mode, once.
  (romacs-agent-plan-tests--with 'claude
    (setf (romacs-agent-options session) '(:plan t :plan-from "bypassPermissions")
          (romacs-agent-driver-state session) (list :initialized t)
          (romacs-agent-permission-mode session) nil)
    (let ((line (romacs-agent--claude-next session)))
      (should (equal (gethash "mode" (gethash "request" line)) "plan"))
      (dolist (event (romacs-agent--claude-input-events session line)) (romacs-agent--raise session event)))
    (should (eq (romacs-agent--claude-next session) :ready))))

(ert-deftest romacs-agent-plan-the-mode-to-go-back-to ()
  (romacs-agent-plan-tests--with 'claude
    ;; Unknown at first: filled in when the harness says.
    (setf (romacs-agent-permission-mode session) nil)
    (romacs-agent-set-plan session t)
    (should-not (plist-get (romacs-agent-options session) :plan-from))
    (romacs-agent--raise session '(:type session :permission-mode "acceptEdits"))
    (should (equal (plist-get (romacs-agent-options session) :plan-from) "acceptEdits")))
  (romacs-agent-plan-tests--with 'claude
    ;; Gone into plan mode of its own: the session follows, its mode before kept.
    (romacs-agent--raise session '(:type mode :permission-mode "plan"))
    (should (plist-get (romacs-agent-options session) :plan))
    (should (equal (plist-get (romacs-agent-options session) :plan-from) "bypassPermissions"))
    ;; Never plan itself as the mode to go back to.
    (romacs-agent-set-plan session nil)
    (should (equal (gethash "mode" (gethash "request" (car written))) "bypassPermissions"))))

(ert-deftest romacs-agent-plan-a-replay-keeps-the-options ()
  ;; A run once in plan mode, replayed: the options (the manifest's last
  ;; word) stay as they are, and nothing is written.
  (romacs-agent-plan-tests--with 'claude
    (setf (romacs-agent-caught-up session) nil)
    (romacs-agent--raise session '(:type mode :permission-mode "plan"))
    (romacs-agent--raise session '(:type mode :permission-mode "bypassPermissions"))
    (should-not (plist-get (romacs-agent-options session) :plan))
    (should-not written)))

(ert-deftest romacs-agent-plan-a-new-run-is-put-in-plan-mode ()
  ;; The last run's mode is not the new one's: reconciled once it is ready.
  (romacs-agent-plan-tests--with 'claude
    (setf (romacs-agent-options session) '(:plan t :plan-from "bypassPermissions")
          (romacs-agent-permission-mode session) "plan")
    (romacs-agent--raise session '(:type run :seq 30))
    (should-not (romacs-agent-permission-mode session))
    (romacs-agent--raise session '(:type handshake :state (:initialized t)))
    (should (equal (gethash "mode" (gethash "request" (romacs-agent--claude-next session))) "plan"))))

(ert-deftest romacs-agent-plan-a-refused-switch-falls-back ()
  (romacs-agent-plan-tests--with 'claude
    (setf (romacs-agent-driver-state session) (list :initialized t :mode-asked "bypassPermissions"))
    (let ((events (romacs-agent--claude-events
                   session (romacs-mcp--object "type" "control_response"
                                               "response" (romacs-mcp--object "subtype" "error" "request_id" "romacs-mode-4"
                                                                              "error" "not allowed")))))
      (should (eq (plist-get (car events) :type) 'error))
      (dolist (event events) (romacs-agent--raise session event)))
    (should (equal (gethash "mode" (gethash "request" (romacs-agent--claude-next session))) "default")))
  ;; Refused into plan mode: the session is not in it.
  (romacs-agent-plan-tests--with 'claude
    (setf (romacs-agent-options session) '(:plan t :plan-from "bypassPermissions")
          (romacs-agent-driver-state session) (list :initialized t :mode-asked "plan"))
    (dolist (event (romacs-agent--claude-events
                    session (romacs-mcp--object "type" "control_response"
                                                "response" (romacs-mcp--object "subtype" "error" "request_id" "romacs-mode-5"
                                                                               "error" "no"))))
      (romacs-agent--raise session event))
    (should-not (plist-get (romacs-agent-options session) :plan))
    (should (eq (romacs-agent--claude-next session) :ready))))

(ert-deftest romacs-agent-plan-tool-calls-are-refused-in-plan-mode ()
  (romacs-agent-plan-tests--with 'claude
    (let ((romacs-agent-approval-policy 'allow))
      (setf (romacs-agent-options session) '(:plan t))
      (puthash "e1" t (romacs-agent-approvals session))
      (romacs-agent--raise session '(:type approval :request-id "e1" :tool "Edit"))
      (should (equal answered (list (list "e1" nil romacs-agent--plan-tool-refused)))))))

(ert-deftest romacs-agent-plan-a-plan-met-on-catching-up-is-captured ()
  (romacs-agent-plan-tests--with 'claude
    (let ((romacs-agent-approval-policy 'allow))
      (setf (romacs-agent-caught-up session) nil)
      (puthash "x9" t (romacs-agent-approvals session))
      (romacs-agent--raise session '(:type approval :request-id "x9" :tool "ExitPlanMode" :plan t :user t))
      (should-not answered)
      (setf (romacs-agent-caught-up session) t)
      (romacs-agent--allow-tool-calls session)
      ;; Stopped, never allowed.
      (should (equal answered (list (list "x9" nil romacs-agent--plan-captured)))))))

(ert-deftest romacs-agent-plan-claude-s-plan-is-captured ()
  (romacs-agent-plan-tests--with 'claude
    (let ((events (romacs-agent--claude-events
                   session (romacs-mcp--object
                            "type" "control_request" "request_id" "x1"
                            "request" (romacs-mcp--object "subtype" "can_use_tool" "tool_name" "ExitPlanMode"
                                                          "input" (romacs-mcp--object "plan" "1. Do it."))))))
      (should (equal (car events) '(:type plan :text "1. Do it.")))
      (dolist (event events) (romacs-agent--raise session event))
      ;; Told to stop, as T3 Code tells it; never left waiting on the user.
      (should (equal answered (list (list "x1" nil romacs-agent--plan-captured))))
      (should-not (eq (romacs-agent-state session) 'waiting-approval)))))

(ert-deftest romacs-agent-plan-codex-says-its-mode-each-turn ()
  (romacs-agent-plan-tests--with 'codex
    (setf (romacs-agent-model session) "gpt-6.1-sol" (romacs-agent-harness-session session) "th")
    (let ((mode (lambda () (gethash "collaborationMode" (gethash "params" (romacs-agent--codex-turn session 1 []))))))
      ;; A Codex not told of the experimental API (started before) refuses
      ;; the turn that says it: none said.
      (should-not (funcall mode))
      (dolist (event (romacs-agent--codex-input-events
                      session (romacs-mcp--object "method" "initialize" "id" 0
                                                  "params" (romacs-mcp--object "capabilities" (romacs-mcp--object "experimentalApi" t)))))
        (romacs-agent--raise session event))
      (should (equal (gethash "mode" (funcall mode)) "default"))
      (should (equal (gethash "model" (gethash "settings" (funcall mode))) "gpt-6.1-sol"))
      (romacs-agent-set-plan session t)
      ;; Nothing to write live: the next turn carries it.
      (should-not written)
      (should (equal (gethash "mode" (funcall mode)) "plan")))))

(ert-deftest romacs-agent-plan-the-card ()
  (with-temp-buffer
    (romacs-agent-chat-mode)
    (let ((session (romacs-agent--make :id "c" :harness 'claude :tools (make-hash-table :test #'equal))))
      (setq romacs-agent-ui--session session romacs-agent-ui--end (point-marker))
      (romacs-agent-ui--draw session '(:type plan :text "1. Read.\n2. Write.") (current-buffer))
      (should (string-match-p "Proposed plan" (buffer-string)))
      (should (string-match-p "1. Read.\n2. Write." (buffer-string)))
      (should (string-match-p "implement it" (buffer-string))))))

(ert-deftest romacs-agent-plan-implement-waits-for-the-turn ()
  (with-temp-buffer
    (romacs-agent-chat-mode)
    (setq romacs-agent-ui--session (romacs-agent--make :id "r" :harness 'claude :state 'running :options '(:plan t)))
    (should-error (romacs-agent-chat-implement) :type 'user-error)))

(provide 'romacs-agent-plan-tests)
;;; romacs-agent-plan-tests.el ends here
