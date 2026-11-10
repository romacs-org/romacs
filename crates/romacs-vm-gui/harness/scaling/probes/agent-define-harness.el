;;; agent-define-harness.el --- scaling probe  -*- lexical-binding: t -*-

;; N harness drivers defined, then each defined again, oldest first (the
;; farthest from the registry's head): each by its name's cell, never by
;; a walk of the registry.
(scaling-probe agent-define-harness :n 2000
  :setup (progn (scaling--agent) (mapcar #'intern (scaling--names "scaling-rdriver" n)))
  :body (progn
          (dolist (name it) (romacs-agent-define-harness name :lifetime 'persistent))
          (dolist (name it) (romacs-agent-define-harness name :lifetime 'per-turn))))
