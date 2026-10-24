;;; -*- lexical-binding: t -*-
(defconst probe-v (<= 2 (length (list 1 2)) 3))
(if (equal probe-v (quote t)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
