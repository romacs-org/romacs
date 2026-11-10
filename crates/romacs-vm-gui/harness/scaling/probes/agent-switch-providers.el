;;; agent-switch-providers.el --- scaling probe  -*- lexical-binding: t -*-

;; The rows of a switcher with N providers configured: a table resolves the
;; default once, not per row.
(scaling-probe agent-switch-providers :n 400 :budget 0.1
  :setup (progn (scaling--agent)
                (require 'romacs-agent-ui)
                (setq romacs-agent-providers
                      (cons (cons "claude" "~/.claude")
                            (mapcar (lambda (name) (cons name "~/.claude")) (scaling--names "provider" n))))
                (romacs-agent-ui--session-names
                 (mapcar (lambda (id) (romacs-agent--make :id id :harness 'claude :provider "claude" :title id
                                                         :state 'idle :cwd "/tmp/"))
                         (scaling--names "scaling-provider" 200))))
  :body (let* ((table (romacs-agent-ui--session-table (car it) (cdr it) 120))
               (rows (alist-get 'affixation-function (cdr (funcall table "" nil 'metadata)))))
          (funcall rows (cdr it))))
