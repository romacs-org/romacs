;;; -*- lexical-binding: t -*-
(defvar probe-n 0)
(eval-and-compile (setq probe-n (1+ probe-n)))
(defconst probe-v probe-n)
(if (equal probe-v (quote 1)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
