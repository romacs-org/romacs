;;; faces-theme-switch.el --- scaling probe  -*- lexical-binding: t -*-

;; A theme switch over N faces the theme sets: `enable-theme' walks them
;; once and recalculates each on the root frame.
(scaling-probe faces-theme-switch :gui t :n 200 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-themed" n))))
           (dolist (face faces) (custom-declare-face face '((t)) "A scaling face." :group 'scaling))
           (deftheme scaling-switch "N faces.")
           (apply #'custom-theme-set-faces 'scaling-switch
                  (mapcar (lambda (face) `(,face ((t :foreground "#a0b0c0" :weight bold)))) faces))
           (switch-to-buffer (get-buffer-create "scaling-switch")))
  ;; None of the faces shows, so the step paints by typing.
  :body (progn (enable-theme 'scaling-switch) (disable-theme 'scaling-switch)
               (with-current-buffer it (insert "x"))))
