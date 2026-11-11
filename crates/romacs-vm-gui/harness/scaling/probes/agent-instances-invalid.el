;;; agent-instances-invalid.el --- scaling probe  -*- lexical-binding: t -*-

;; N entries naming no harness Romacs runs, with N drivers defined: each a
;; short reason; the harnesses there are listed once.
(scaling-probe agent-instances-invalid :n 500
  :setup (progn (scaling--agent)
                (dotimes (i n)
                  (romacs-agent-define-harness (intern (format "scaling-idriver-%d" i)) :lifetime 'persistent
                                               :config-variable (format "SCALING_IDRIVER_%d_HOME" i)))
                (cl-loop for i below n collect (list (format "nosuch-%d" i) :harness (intern (format "nosuch-%d" i)))))
  :body (let ((romacs-agent-providers it)) (romacs-agent-instances t)))
