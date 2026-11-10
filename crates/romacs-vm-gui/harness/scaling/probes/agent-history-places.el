;;; agent-history-places.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-history-places :n 1000
  :setup (progn (scaling--agent) (require 'romacs-agent-history)
                (cl-loop for i below n collect (cons (format "claude-%d" i) (format "/tmp/claude-config-%d" i))))
  :body (let ((romacs-agent-providers it)) (romacs-agent-history--places)))
