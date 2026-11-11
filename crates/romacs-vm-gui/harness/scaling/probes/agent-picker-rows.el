;;; agent-picker-rows.el --- scaling probe  -*- lexical-binding: t -*-

;; The start palette of N instances, each with the harness's catalogue:
;; rows by instance, one table of shown names.
(scaling-probe agent-picker-rows :n 1000
  :setup (progn (scaling--agent)
                (cl-loop for i below n collect (list (format "claude-%d" i) :display "Claude"
                                                     :config-dir (format "/tmp/claude-config-%d" i))))
  :body (let ((romacs-agent-providers it)) (romacs-agent--picker-rows)))
