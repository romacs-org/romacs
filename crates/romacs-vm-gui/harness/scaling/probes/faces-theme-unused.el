;;; faces-theme-unused.el --- scaling probe  -*- lexical-binding: t -*-

;; A theme switch over N package faces no theme mentions, each with a
;; spec that depends on the background mode.
(scaling-probe faces-theme-unused :gui t :n 200 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-unused" n))))
           (dolist (face faces)
             (custom-declare-face face '((((background dark)) :foreground "#aabbcc") (t :foreground "#112233"))
                                  "A package face." :group 'scaling))
           (deftheme scaling-small "One face.")
           (custom-theme-set-faces 'scaling-small '(default ((t :background "#101010" :foreground "#e0e0e0"))))
           faces)
  :body (progn (enable-theme 'scaling-small) (disable-theme 'scaling-small)))
