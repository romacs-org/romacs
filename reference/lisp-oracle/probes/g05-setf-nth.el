;;; -*- lexical-binding: t -*-
(defconst probe-v (let ((l (list 0 1 2 3 4 5 6))) (setf (nth 5 l) 9) l))
(if (equal probe-v (quote (0 1 2 3 4 9 6))) (defconst probe-ok t) (signal (quote error) (list (quote probe-mismatch) probe-v)))
