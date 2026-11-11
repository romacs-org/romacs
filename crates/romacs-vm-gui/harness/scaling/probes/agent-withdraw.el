;;; agent-withdraw.el --- scaling probe  -*- lexical-binding: t -*-

;; N queued calls, each withdrawn by its relay.
(scaling-probe agent-withdraw :n 64
  :setup (progn (require 'romacs-mcp) (number-sequence 0 (1- n)))
  :body (let ((romacs-mcp--queue nil) (romacs-mcp--queued (make-hash-table :test #'equal)))
          (dolist (i it)
            (let ((request (list :peer 'scaling-peer :message (romacs-mcp--object "id" i) :cancelled nil)))
              (push request romacs-mcp--queue)
              (puthash (cons 'scaling-peer i) request romacs-mcp--queued)))
          (cl-letf (((symbol-function 'romacs-mcp--send) #'ignore))
            (dolist (i it) (romacs-mcp--withdraw 'scaling-peer "w" i)))))
