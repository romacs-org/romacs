;;; options-set.el --- scaling probe  -*- lexical-binding: t -*-

;; Each option set again, at N settings of the `user' theme, older entries too.
(scaling-probe options-set :n 500
  :known "ticket custom-push-theme: GNU's scan of theme-settings on each change of a setting"
  :setup (let ((options (scaling--declare-options "scaling-set" n)))
           (dolist (option options) (romacs-option-set option 1))
           options)
  :body (dolist (option it) (romacs-option-set option 2)))
