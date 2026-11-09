;;; agent-children.el --- scaling probe  -*- lexical-binding: t -*-

;; What runs under a session of N delegated tasks: rows and a summary, made
;; once per batch of their events, however many arrive.
(scaling-probe agent-children :n 1000
  :setup (progn (scaling--agent) (require 'romacs-agent-ui)
                (let ((parent (romacs-agent--make :id "scaling-parent" :harness 'claude :provider "claude")))
                  (puthash "scaling-parent" parent romacs-agent--sessions)
                  (dotimes (i n)
                    (let ((child (romacs-agent--make :id (format "scaling-child-%d-%d" n i) :harness 'codex
                                                     :provider "codex" :state 'running
                                                     :options '(:parent "scaling-parent"))))
                      (puthash (romacs-agent-id child) child romacs-agent--sessions)
                      (romacs-agent--index child)))
                  (romacs-agent-children parent)))
  :body (progn
          (dolist (child it)
            (romacs-agent-ui--track-children child (list :type 'done)))
          (romacs-agent-ui--children-flush)))
