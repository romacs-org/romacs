;;; attach-tray.el --- scaling probe  -*- lexical-binding: t -*-

;; One batch insertion of N items: planned and rendered once, linear.
(scaling-probe attach-tray :gui t :n 100 :steps 1
  :setup (progn (require 'romacs-attach) (cons (scaling--attach-host nil) (scaling--attach-files n)))
  :body (with-current-buffer (car it) (romacs-attach-set-items nil) (romacs-attach-files (cdr it))))
