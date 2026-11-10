;;; theme-preview-owned-frames.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe theme-preview-owned-frames :gui t :n 8 :steps 1
  :setup (progn
           (scaling--preview-theme)
           (scaling--frames n t)
           (switch-to-buffer (get-buffer-create "scaling-owned")))
  :body (scaling--preview-once it))
