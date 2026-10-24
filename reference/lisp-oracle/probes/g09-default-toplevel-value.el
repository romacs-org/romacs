;;; -*- lexical-binding: t -*-
(defconst probe-v (default-toplevel-value (quote lexical-binding)))
(if (equal probe-v (quote nil)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
