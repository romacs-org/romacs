;;; vertical-motion-up.el --- scaling probe  -*- lexical-binding: t -*-

;; `vertical-motion' up N lines, as `recenter' does to place point: each
;; line walked once, not once for each line stepped back.
(scaling-probe vertical-motion-up :gui t :n 200 :k 8 :steps 1
  :setup (switch-to-buffer (scaling--lines-buffer "scaling-motion" (* 2 n)))
  :body (with-current-buffer it
          (goto-char (point-max))
          (vertical-motion (- n))))
