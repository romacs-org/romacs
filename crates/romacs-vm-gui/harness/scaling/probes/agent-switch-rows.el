;;; agent-switch-rows.el --- scaling probe  -*- lexical-binding: t -*-

;; The switcher's rows for N sessions, as a prompt shows every match.
(scaling-probe agent-switch-rows :n 2000
  :setup (progn (scaling--agent)
                (require 'romacs-agent-ui)
                (romacs-agent-ui--session-names
                 (mapcar (lambda (id) (romacs-agent--make :id id :harness 'claude :provider "claude" :title id
                                                         :state 'idle :cwd "/tmp/"))
                         (scaling--names "scaling-session" n))))
  :body (let* ((table (romacs-agent-ui--session-table (car it) (cdr it) 120))
               (rows (alist-get 'affixation-function (cdr (funcall table "" nil 'metadata)))))
          (funcall rows (cdr it))))
