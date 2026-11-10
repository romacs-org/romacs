;;; faces-theme-frames.el --- scaling probe  -*- lexical-binding: t -*-

;; A theme switch with N child frames open: they show the root's faces,
;; so what the switch adds per frame is a check that it shares.
(scaling-probe faces-theme-frames :gui t :n 8 :steps 1
  :setup (progn
           (deftheme scaling-small "One face.")
           (custom-theme-set-faces 'scaling-small '(default ((t :background "#101010" :foreground "#e0e0e0"))))
           (unless (fboundp 'pgtk-register-dnd-targets) (defalias 'pgtk-register-dnd-targets #'ignore))
           (dotimes (_ n) (make-frame `((parent-frame . ,(selected-frame)) (visibility . nil) (minibuffer . nil))))
           n)
  :body (progn (enable-theme 'scaling-small) (disable-theme 'scaling-small)))
