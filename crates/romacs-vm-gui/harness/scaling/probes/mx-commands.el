;;; mx-commands.el --- scaling probe  -*- lexical-binding: t -*-

;; Fifty keys of M-x after its first, over N more commands: what each key
;; costs before matching, M-x's candidates kept while nothing they depend on
;; changed (lisp/romacs/romacs-commands.el).  No predicate is asked about
;; each command, so the work stays flat.  Matching the names stays linear
;; and native, as GNU's `all-completions' is.
(scaling-probe mx-commands :n 20000 :budget 0.1
  :setup (progn
           (setq read-extended-command-predicate #'command-completion-default-include-p)
           (dolist (name (scaling--names "scaling-mx" n))
             (defalias (intern name) (lambda () (interactive))))
           (let ((pred (romacs-commands--predicate (current-buffer))))
             (all-completions "" (romacs-commands--table pred (current-buffer)) pred)
             pred))
  :body (dotimes (_ 50) (romacs-commands--candidates it (current-buffer))))
