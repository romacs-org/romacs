;;; romacs-agent-approval-tests.el --- Answering what a harness asks -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-claude)
  (require 'romacs-agent-ui))

;; What Claude Code 2.1.316 sent for an AskUserQuestion, with
;; --permission-prompt-tool stdio, under bypassPermissions.
(defconst romacs-agent-approval-tests--ask
  "{\"type\":\"control_request\",\"request_id\":\"q1\",\"request\":{\"subtype\":\"can_use_tool\",\"tool_name\":\"AskUserQuestion\",\"display_name\":\"AskUserQuestion\",\"input\":{\"questions\":[{\"question\":\"Which colour do you prefer?\",\"header\":\"Colour\",\"options\":[{\"label\":\"Red\",\"description\":\"Choose red.\"},{\"label\":\"Blue\",\"description\":\"Choose blue.\"}],\"multiSelect\":false}]},\"tool_use_id\":\"toolu_1\",\"requires_user_interaction\":true}}")

(defconst romacs-agent-approval-tests--bash
  "{\"type\":\"control_request\",\"request_id\":\"b1\",\"request\":{\"subtype\":\"can_use_tool\",\"tool_name\":\"Bash\",\"input\":{\"command\":\"make\"},\"permission_suggestions\":[{\"type\":\"addRules\",\"rules\":[{\"toolName\":\"Bash\",\"ruleContent\":\"make:*\"}],\"behavior\":\"allow\",\"destination\":\"localSettings\"}]}}")

(defun romacs-agent-approval-tests--session ()
  (romacs-agent--make :id "s" :harness 'claude :tools (make-hash-table :test #'equal)
                      :streamed (make-hash-table :test #'equal)))

(defun romacs-agent-approval-tests--response (object)
  (gethash "response" (gethash "response" object)))

(ert-deftest romacs-agent-approval-question-is-asked ()
  (let* ((session (romacs-agent-approval-tests--session))
         (event (car (romacs-agent--claude-events session (json-parse-string romacs-agent-approval-tests--ask)))))
    (should (eq (plist-get event :type) 'approval))
    (should (equal (plist-get event :questions)
                   '(("Which colour do you prefer?" :header "Colour" :options ("Red" "Blue")
                      :notes (("Red" . "Choose red.") ("Blue" . "Choose blue.")) :multi nil))))
    ;; Answered: the input as asked, the answers beside it.
    (let* ((response (romacs-agent-approval-tests--response
                      (romacs-agent--claude-respond session "q1" '(:answers (("Which colour do you prefer?" . "Blue"))) nil)))
           (input (gethash "updatedInput" response)))
      (should (equal (gethash "behavior" response) "allow"))
      (should (equal (gethash "Which colour do you prefer?" (gethash "answers" input)) "Blue"))
      (should (= (length (gethash "questions" input)) 1))
      ;; The request's own input is untouched.
      (should-not (gethash "answers" (gethash "input" (gethash "q1" (romacs-agent-approvals session))))))))

(ert-deftest romacs-agent-approval-for-the-session ()
  (let ((session (romacs-agent-approval-tests--session)))
    (romacs-agent--claude-events session (json-parse-string romacs-agent-approval-tests--bash))
    (should-not (plist-get (car (romacs-agent--claude-events session (json-parse-string romacs-agent-approval-tests--bash)))
                           :questions))
    (let* ((response (romacs-agent-approval-tests--response (romacs-agent--claude-respond session "b1" '(:always t) nil)))
           (update (aref (gethash "updatedPermissions" response) 0)))
      ;; Claude Code's suggestion, kept to the session.
      (should (equal (gethash "destination" update) "session"))
      (should (equal (gethash "ruleContent" (aref (gethash "rules" update) 0)) "make:*")))
    ;; As asked; denied.
    (should-not (gethash "updatedPermissions" (romacs-agent-approval-tests--response
                                               (romacs-agent--claude-respond session "b1" t nil))))
    (should (equal (gethash "behavior" (romacs-agent-approval-tests--response
                                        (romacs-agent--claude-respond session "b1" nil "no")))
                   "deny"))
    ;; No suggestion: the tool itself.
    (puthash "b1" (romacs-mcp--object "tool_name" "Edit" "input" (romacs-mcp--object)) (romacs-agent-approvals session))
    (let ((rule (aref (gethash "rules" (aref (gethash "updatedPermissions"
                                                      (romacs-agent-approval-tests--response
                                                       (romacs-agent--claude-respond session "b1" '(:always t) nil)))
                                             0))
                      0)))
      (should (equal (gethash "toolName" rule) "Edit")))))

(ert-deftest romacs-agent-approval-chat-reads-the-answers ()
  (let* ((session (romacs-agent-approval-tests--session))
         (event (car (romacs-agent--claude-events session (json-parse-string romacs-agent-approval-tests--ask))))
         (given nil))
    (with-temp-buffer
      (setq romacs-agent-ui--session session)
      (puthash "q1" event (romacs-agent-ui--asks))
      (cl-letf (((symbol-function 'completing-read) (lambda (prompt table &rest _)
                                                      (should (string-match-p "Which colour" prompt))
                                                      (should (equal (all-completions "" table) '("Red" "Blue")))
                                                      ;; Each option with what it means.
                                                      (let ((note (alist-get 'annotation-function
                                                                             (cdr (funcall table "" nil 'metadata)))))
                                                        (should (string-match-p "Choose blue" (funcall note "Blue"))))
                                                      "Blue"))
                ((symbol-function 'romacs-agent-respond) (lambda (_s id allow &rest _) (setq given (list id allow)))))
        (should (romacs-agent-ui--asking-p))
        (should (equal (romacs-agent-ui--allow-label) "Answer"))
        (romacs-agent-chat-allow)
        (should (equal given '("q1" (:answers (("Which colour do you prefer?" . "Blue"))))))
        ;; ! on a question answers it too: no rule for the session.
        (setq given nil)
        (romacs-agent-chat-allow-always)
        (should (equal given '("q1" (:answers (("Which colour do you prefer?" . "Blue"))))))))))

(provide 'romacs-agent-approval-tests)
;;; romacs-agent-approval-tests.el ends here
