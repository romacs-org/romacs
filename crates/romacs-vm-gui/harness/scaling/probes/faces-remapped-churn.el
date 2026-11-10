;;; faces-remapped-churn.el --- scaling probe  -*- lexical-binding: t -*-

;; Distinct anonymous faces in a buffer with a remapping, past the size a
;; table is kept to.
(scaling-probe faces-remapped-churn :gui t :n 1500 :steps 1
  :setup (let ((buffer (switch-to-buffer (get-buffer-create "scaling-remapped"))))
           (setq-local face-remapping-alist '((default :height 1.5)))
           (dotimes (i n)
             (insert (propertize "x" 'face (list :foreground (format "#%06x" (* 3 (1+ i)))))))
           buffer)
  :body (with-current-buffer it (goto-char (point-max)) (insert "x")))
