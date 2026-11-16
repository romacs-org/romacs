;;; recenter-long-line.el --- scaling probe  -*- lexical-binding: t -*-

;; `recenter' with point on the line after one logical line N
;; characters long, which wraps into many rows above it, each step
;; painted.  Its rows come
;; from laying that line out from its start, as GNU's iterator walks it
;; from its start: linear in the line, never worse.
(scaling-probe recenter-long-line :gui t :n 4000 :k 8 :steps 4
  :setup (let ((buffer (get-buffer-create "scaling-recenter-long")))
           (switch-to-buffer buffer)
           (erase-buffer)
           (insert "head\n" (mapconcat #'identity (make-list (/ n 5) "word") " ") "\ntail\n")
           buffer)
  :body (with-current-buffer it
          (goto-char (point-min))
          (forward-line 2)
          (recenter (if (cl-evenp i) nil -1))))
