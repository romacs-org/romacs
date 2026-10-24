;;; -*- lexical-binding: t -*-
(defconst probe-v (list #o040 #o0177))
(if (equal probe-v (quote (32 127))) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
