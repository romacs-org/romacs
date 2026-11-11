;;; windows.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe windows :n 6
  :setup (progn
           (setq window-min-height 1 window-min-width 2)
           (delete-other-windows)
           nil)
  :body (progn
          (dotimes (i n)
            (split-window (get-largest-window) nil (if (cl-oddp i) 'right 'below)))
          (let ((count 0)) (walk-windows (lambda (_) (setq count (1+ count)))) count)
          (dolist (w (window-list)) (ignore-errors (window-resize w 1 nil)))
          (balance-windows)
          (delete-other-windows)))
