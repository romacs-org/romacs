;; -*- lexical-binding: t -*-
(dolist (form '((set 'most-positive-fixnum 0) (makunbound 'most-negative-fixnum) (make-local-variable 'most-positive-fixnum)))
  (prin1 (condition-case e (list 'unexpected (eval form t)) (setting-constant (car e)))) (terpri))
(with-temp-buffer
  (let ((b2 (generate-new-buffer "s124-oracle-mode-b")) (b3 (generate-new-buffer "s124-oracle-mode-c")))
    (unwind-protect
        (progn (make-local-variable 'major-mode) (set-buffer b2) (emacs-lisp-mode) (set-buffer b3) (prin1 major-mode) (terpri))
      (kill-buffer b2) (kill-buffer b3))))
(setq s124-oracle-x 1)
(with-temp-buffer
  (make-local-variable 's124-oracle-x)
  (prin1 (local-variable-if-set-p 's124-oracle-x)) (terpri)
  (with-temp-buffer (prin1 (local-variable-if-set-p 's124-oracle-x)) (terpri)))
