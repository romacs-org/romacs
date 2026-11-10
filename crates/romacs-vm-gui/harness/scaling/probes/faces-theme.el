;;; faces-theme.el --- scaling probe  -*- lexical-binding: t -*-

;; A theme application over N more package faces: loading and enabling
;; catppuccin-latte in a -Q window, most of it loading the theme file; nearly flat in the faces, which only the root frame
;; recomputes (VM2-CHILD-FRAME).
(scaling-probe faces-theme :gui t :n 50 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces)
             (custom-declare-face face '((t :inherit font-lock-keyword-face)) "A package face." :group 'scaling))
           faces)
  :body (load-theme 'catppuccin-latte t))
