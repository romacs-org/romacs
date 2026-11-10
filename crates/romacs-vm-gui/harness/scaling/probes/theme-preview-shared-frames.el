;;; theme-preview-shared-frames.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe theme-preview-shared-frames :gui t :n 8 :steps 1
  :setup (progn
           (scaling--preview-theme)
           (scaling--frames n nil)
           (switch-to-buffer (get-buffer-create "scaling-shared")))
  :body (scaling--preview-once it))
