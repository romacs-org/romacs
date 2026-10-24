;;; -*- lexical-binding: t -*-
(defconst probe-v (logior 128 ?\M-\C-@))
(if (equal probe-v (quote 134217856)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
