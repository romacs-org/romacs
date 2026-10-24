;;; -*- lexical-binding: t -*-
(defconst probe-v (eval (quote (let ((probe-zz 1)) (funcall (lambda () probe-zz))))))
(if (equal probe-v (quote 1)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
