;;; gnosis-agent-romacs.el --- Gnosis evaluation and tutoring through Romacs agent sessions -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; `gnosis-agent-eval-function' and `gnosis-agent-review-function' through
;; Romacs's agent sessions (romacs-agent.el): each request is one
;; isolated session under a provider instance (Claude Code, Codex) and the
;; model the user chose, its prompt and its answer's validation those of
;; the Hermes adapters (same protocol, same refusals).  Like them, this
;; never reads a database, accepts a grade or retries: a malformed answer
;; is rejected and the durable core keeps the response for another try.

;;; Code:

(require 'gnosis-agent-eval-hermes)
(require 'gnosis-agent-review-hermes)

(declare-function romacs-agent-ask "romacs-agent" (prompt resolve reject &rest options))

(defgroup gnosis-agent-romacs nil
  "Gnosis evaluation and tutoring through Romacs agent sessions."
  :group 'gnosis)

(defcustom gnosis-agent-romacs-provider nil
  "The provider instance (a name in `romacs-agent-providers') that
evaluates and tutors, or nil for `romacs-agent-default-provider'."
  :type '(choice (const :tag "Default instance" nil) string)
  :group 'gnosis-agent-romacs)

(defcustom gnosis-agent-romacs-model nil
  "The model the evaluating session runs, or nil for the instance's own."
  :type '(choice (const :tag "Instance default" nil) string)
  :group 'gnosis-agent-romacs)

(defcustom gnosis-agent-romacs-effort nil
  "The effort the evaluating session runs at, or nil for the harness's own."
  :type '(choice (const :tag "Harness default" nil) string)
  :group 'gnosis-agent-romacs)

(defun gnosis-agent-romacs--ask (prompt parse resolve reject title)
  "Ask PROMPT of one session; RESOLVE with PARSE's result or REJECT once."
  (require 'romacs-agent)
  (romacs-agent-ask prompt
                    (lambda (text)
                      (let ((result (condition-case nil (funcall parse text) (error nil))))
                        (if result (funcall resolve result)
                          (funcall reject "Invalid evaluator response; no grade recorded"))))
                    reject
                    :provider gnosis-agent-romacs-provider :model gnosis-agent-romacs-model
                    :effort gnosis-agent-romacs-effort :title title))

;;;###autoload
(defun gnosis-agent-eval-romacs (request resolve reject)
  "Evaluate captured REQUEST in a Romacs agent session; RESOLVE or REJECT once.
As `gnosis-agent-eval-hermes': REQUEST holds :question, :reference-answer,
:rubric and :response strings; RESOLVE gets (:verdict pass|fail|ungradable
:explanation STRING).  Return an idempotent cancellation function.  No
grade is recorded."
  (gnosis-agent-romacs--ask (gnosis-agent-eval-hermes--prompt request) #'gnosis-agent-eval-hermes--parse
                            resolve reject "gnosis evaluation"))

;;;###autoload
(defun gnosis-agent-review-romacs (request resolve reject)
  "Send contextual REQUEST to a Romacs agent session; RESOLVE or REJECT once.
As `gnosis-agent-review-hermes': the same tutor protocol and the same
validation of its answer.  Return an idempotent cancellation function."
  (gnosis-agent-romacs--ask (gnosis-agent-review-hermes--prompt request) #'gnosis-agent-review-hermes--parse
                            resolve reject "gnosis tutor"))

(provide 'gnosis-agent-romacs)
;;; gnosis-agent-romacs.el ends here
