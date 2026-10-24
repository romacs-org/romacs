;;; -*- lexical-binding: t -*-
(defun va-show (form) (princ (format "%S\n" (condition-case e (list 'ok (eval form t)) (error (list 'sig e))))))
(dolist (f '((progn (defvar va-bb 1) (make-variable-buffer-local 'va-bb) (defvaralias 'va-ba 'va-bb))
             (with-temp-buffer (list (let ((va-ba 8)) (setq va-ba 9) va-bb) va-bb))
             (with-temp-buffer (list (let ((va-bb 8)) (setq va-bb 9) va-bb) va-bb))))
  (va-show f))
