;;; -*- lexical-binding: t -*-
(defconst probe-v (let (r) (pcase-dolist (`(,a ,b) (list (list 1 2))) (push (+ a b) r)) r))
(if (equal probe-v (quote (3))) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
