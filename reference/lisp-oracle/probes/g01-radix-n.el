;;; -*- lexical-binding: t -*-
(defconst probe-v #24r1k)
(if (equal probe-v (quote 44)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
