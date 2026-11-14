;;; display-idle-tick.el --- scaling probe  -*- lexical-binding: t -*-

;; A timer that changes nothing and the redisplay after it, with N windows
;; on screen, 50 N more live buffers and N child frames that are not shown.
;; Today the work grows with them, exponent 0.4 from 4 to 16: each window
;; on screen is visited (its key built, its bands expanded), and every
;; frame's windows are walked before redisplay.  The budget is what that
;; costs; a redisplay that visits only what changed lowers it to 0.  The
;; windows are a grid, so N is a square.
(scaling-probe display-idle-tick :gui t :idle t :n 4 :k 4 :steps 20 :budget 0.5
  :setup (let ((side (round (sqrt n))) (i 0))
           (dotimes (j (* 50 n)) (scaling--lines-buffer (format "scaling-idle-%d" j) 20))
           (scaling--frames n nil)
           (delete-other-windows)
           (dotimes (_ (1- side)) (split-window-right))
           (dolist (window (window-list))
             (with-selected-window window (dotimes (_ (1- side)) (split-window-below))))
           (balance-windows)
           (dolist (window (window-list))
             (set-window-buffer window (format "scaling-idle-%d" i))
             (setq i (1+ i))))
  :body nil)
