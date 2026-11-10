;;; icon-sizes.el --- scaling probe  -*- lexical-binding: t -*-

;; Text showing an icon at N pixel sizes, laid out each step: the font
;; resolver's per-icon answers (`resolve_sole'), one per size, are found,
;; not resolved again.
(scaling-probe icon-sizes :gui t :n 20 :steps 20
  :setup (let ((buffer (scaling--lines-buffer "scaling-icon-sizes" 10)))
           (with-current-buffer buffer
             (goto-char (point-min))
             (dotimes (i n)
               (insert (propertize (string #xFE001) 'face (list :height (+ 1.0 (* 0.01 i)))) " ")))
           (switch-to-buffer buffer)
           buffer)
  :body (with-current-buffer it (goto-char (point-max)) (insert "x")))
