;;; -*- lexical-binding: t -*-
(defun review-read (s) (if (boundp s) (symbol-value s) 'unbound))


(defvar review-global 1)
(defvar review-global)
(defun review-local-declaration () (defvar review-function-local))
(review-local-declaration)
(setq review-checks
 (list (special-variable-p 'nil) (special-variable-p 't)
       (let ((review-global 9)) (review-read 'review-global))
       (let ((review-function-local 7)) (review-read 'review-function-local))))
review-checks
