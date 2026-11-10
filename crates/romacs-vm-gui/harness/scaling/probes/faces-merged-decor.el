;;; faces-merged-decor.el --- scaling probe  -*- lexical-binding: t -*-

;; N faces differing only in an underline's colour, merged for text.
(scaling-probe faces-merged-decor :gui t :n 200 :steps 1
  :setup (let ((buffer (switch-to-buffer (get-buffer-create "scaling-decor"))))
           (dotimes (i n)
             (insert (propertize "x" 'face (list :underline (list :color (format "#%06x" (* 7 (1+ i))))))))
           buffer)
  :body (with-current-buffer it (goto-char (point-max)) (insert "x")))
