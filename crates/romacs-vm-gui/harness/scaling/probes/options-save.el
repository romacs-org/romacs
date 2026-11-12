;;; options-save.el --- scaling probe  -*- lexical-binding: t -*-

;; Five saves at N saved settings: each is GNU's whole-file rewrite.  The
;; setup saves each setting as `customize-save-variable' does and writes
;; the file once: a rewrite per setting is quadratic in N, and cost a
;; hundred times the work the probe measures.
(scaling-probe options-save :n 300
  :setup (let ((options (scaling--declare-options "scaling-save" n)))
           (scaling--saving-session)
           (cl-letf (((symbol-function 'custom-save-all) #'ignore))
             (dolist (option options) (customize-save-variable option 1)))
           (custom-save-all)
           options)
  :body (dotimes (i 5) (romacs-option-save (car it) (+ i 2))))
