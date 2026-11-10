;;; options-save.el --- scaling probe  -*- lexical-binding: t -*-

;; Five saves at N saved settings: each is GNU's whole-file rewrite.
(scaling-probe options-save :n 300
  :setup (let ((options (scaling--declare-options "scaling-save" n)))
           (scaling--saving-session)
           (dolist (option options) (customize-save-variable option 1))
           options)
  :body (dotimes (i 5) (romacs-option-save (car it) (+ i 2))))
