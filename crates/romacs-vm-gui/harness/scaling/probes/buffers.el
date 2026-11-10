;;; buffers.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe buffers :n 300
  :known "ticket buffers: creating or killing a buffer copies and rebuilds the whole buffer order list"
  :setup (scaling--names "scaling-buffer" n)
  :body (progn
          (dolist (name it) (get-buffer-create name))
          (dolist (name it) (get-buffer name))
          (length (buffer-list))
          (dolist (name it) (kill-buffer name))))
