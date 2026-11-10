;;; redisplay-windows.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe redisplay-windows :gui t :n 3 :steps 20 :gap 0.4
  :setup (let ((buffer (scaling--lines-buffer "scaling-windows" 200)))
           (setq window-min-height 1 window-min-width 2)
           (switch-to-buffer buffer)
           (delete-other-windows)
           (dotimes (i n) (split-window (get-largest-window) nil (if (cl-oddp i) 'right 'below)))
           (balance-windows)
           buffer)
  :body (with-current-buffer it (goto-char (point-min)) (forward-line i) (insert "x")))
