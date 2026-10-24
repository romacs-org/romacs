;;; -*- lexical-binding: t -*-
(defconst probe-v ?\M-\C-@)
(if (equal probe-v (quote 134217728)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
