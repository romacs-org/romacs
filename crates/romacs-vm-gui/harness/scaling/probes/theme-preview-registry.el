;;; theme-preview-registry.el --- scaling probe  -*- lexical-binding: t -*-

;; N is the registry, faces times frames with their own: what is saved is
;; that much, on both sides of the product at once.
(scaling-probe theme-preview-registry :gui t :n 400 :steps 1
  :setup (let ((side (floor (sqrt n))))
           (dolist (face (mapcar #'intern (scaling--names "scaling-registry" side)))
             (custom-declare-face face '((t :foreground "#a0b0c0")) "A scaling face." :group 'scaling))
           (scaling--preview-theme)
           (scaling--frames side t)
           (switch-to-buffer (get-buffer-create "scaling-registry")))
  :body (scaling--preview-once it))
