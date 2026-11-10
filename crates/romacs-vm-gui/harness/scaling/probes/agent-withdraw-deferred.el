;;; agent-withdraw-deferred.el --- scaling probe  -*- lexical-binding: t -*-

;; N deferred calls (delegations waiting) withdrawn by their relay, then
;; retired by the next poll.
(scaling-probe agent-withdraw-deferred :n 64
  :setup (progn (require 'romacs-mcp) (number-sequence 0 (1- n)))
  ;; An inactive timer: none is scheduled, and retiring the last call
  ;; cancels it as it would the real one.
  :body (let ((romacs-mcp--deferred nil) (romacs-mcp--deferred-index (make-hash-table :test #'equal))
              (romacs-mcp--deferred-timer (timer-create)))
          (dolist (i it)
            (romacs-mcp--defer-request (list :peer 'scaling-peer :cancelled nil) i #'ignore nil))
          (cl-letf (((symbol-function 'romacs-mcp--send) #'ignore))
            (dolist (i it) (romacs-mcp--withdraw 'scaling-peer "w" i))
            (romacs-mcp--poll-deferred))
          (unless (null romacs-mcp--deferred) (error "Withdrawn calls were not retired"))))
