;;; theme-preview-faces.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe theme-preview-faces :gui t :n 200 :steps 1
  :setup (progn
           (dolist (face (mapcar #'intern (scaling--names "scaling-saved" n)))
             (custom-declare-face face '((t :foreground "#a0b0c0")) "A scaling face." :group 'scaling))
           (scaling--preview-theme)
           (switch-to-buffer (get-buffer-create "scaling-saved")))
  :body (scaling--preview-once it))
