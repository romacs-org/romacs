;;; -*- lexical-binding: t -*-
(defconst probe-v (let ((lexical-binding t)) lexical-binding))
(if (equal probe-v (quote t)) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
