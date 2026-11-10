;;; theme-inherit-depth.el --- scaling probe  -*- lexical-binding: t -*-

;; The floor pass over an `:inherit' chain N deep: each ancestor is read once.
(scaling-probe theme-inherit-depth :n 2000
  :setup (progn
           (require 'romacs-theme-roles)
           (let ((names (mapcar #'intern (scaling--names "scaling-inherit" n))) (rows nil) (parent nil))
             (dolist (name names (nreverse rows))
               (push (list name (list (if parent
                                          (list t :inherit parent :background "#101418")
                                        (list t :foreground "#eeeeee"))))
                     rows)
               (setq parent name))))
  :body (romacs-theme-roles-floors it '((bg . "#101418"))))
