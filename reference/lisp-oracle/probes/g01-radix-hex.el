;;; -*- lexical-binding: t -*-
(defconst probe-v #x20)
(if (equal probe-v (quote 32)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
