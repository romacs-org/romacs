;;; toasts-follow.el --- scaling probe  -*- lexical-binding: t -*-

;; N toasts following the selection between two windows, with N more
;; layers kept hidden: one restack of N toasts per change, for the root
;; frame only, never one per child frame's own window change.
(scaling-probe toasts-follow :gui t :n 10 :steps 10
  :setup (progn (dotimes (i n) (romacs-layer--frame (get-buffer-create (format " scaling-layer-%d" i))))
                (delete-other-windows) (split-window-right)
                (setq romacs-layer-toast-max n romacs-layer-toast-timeout 600 romacs-layer-toast-position 'window)
                (dotimes (i n) (romacs-layer-toast (format "toast %d" i)))
                n)
  :body (select-window (next-window)))
