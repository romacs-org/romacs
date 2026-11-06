;;; late-file-test.el --- let-binding a variable made special earlier in the same form  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; For late_special_eval.rs: each `progn' makes its variable special, then
;; let-binds it; GNU binds it dynamically loading this as .el and as .elc.
(defvar late-file-results nil)
(progn (require 'late-file-lib)
       (defun late-file-get () late-file-var)
       (push (list 'require (let ((late-file-var 2)) (late-file-get))) late-file-results)
       (defun late-file-bind () (let ((late-file-var 3)) (late-file-get))))
(progn (defvar late-file-x 1)
       (defun late-file-x-get () late-file-x)
       (push (list 'defvar (let ((late-file-x 2)) (late-file-x-get))) late-file-results))
(push (list 'later-call (late-file-bind)) late-file-results)
(provide 'late-file-test)
