;;; late-file-early.el --- a function let-binding a variable declared later  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; For late_special_eval.rs: GNU's interpreter, loading this as source,
;; binds `late-early-v' dynamically once its `defvar' ran; byte-compiled,
;; the function binds it lexically, as bytecomp compiled it before.
(defun late-early-g () (if (boundp 'late-early-v) late-early-v 'void))
(defun late-early-f () (let ((late-early-v 2)) (late-early-g)))
(defvar late-early-v 1)
(defvar late-early-result (late-early-f))
(provide 'late-file-early)
