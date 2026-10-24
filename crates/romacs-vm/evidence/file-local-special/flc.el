;;; flc.el --- -*- lexical-binding: t -*-
(defun flc-get (s) (if (boundp s) (symbol-value s) 'unbound))
(progn (defvar flc-p))
(when t (defvar flc-w))
(let () (defvar flc-l))
(when nil (defvar flc-n))
(setq flc-r (list (let ((flc-p 1)) (flc-get 'flc-p)) (let ((flc-w 2)) (flc-get 'flc-w)) (let ((flc-l 3)) (flc-get 'flc-l)) (let ((flc-n 4)) (flc-get 'flc-n))))
