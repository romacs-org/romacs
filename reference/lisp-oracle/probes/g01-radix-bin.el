;;; -*- lexical-binding: t -*-
(defconst probe-v #b101)
(if (equal probe-v (quote 5)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
