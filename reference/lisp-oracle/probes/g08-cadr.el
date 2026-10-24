;;; -*- lexical-binding: t -*-
(defconst probe-v (cadr (list 1 2)))
(if (equal probe-v (quote 2)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
