;;; flq.el --- -*- lexical-binding: t -*-
(defvar flp-x 0)
(setq flp-1 (funcall (lambda (flp-x) (flp-get)) 42))
(setq flp-2 (let ((flp-x 43)) (flp-get)))
