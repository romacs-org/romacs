;;; images-in-buffer.el --- scaling probe  -*- lexical-binding: t -*-

;; N lines each with its own image, shown a window at a time: a step's
;; redisplay decodes and lays out the images it shows, so the work is in
;; the window's lines, not the buffer's images.
(scaling-probe images-in-buffer :gui t :n 200 :steps 10
  :setup (progn (clear-image-cache t) (scaling--image-buffer n))
  :body (with-current-buffer it (forward-line 15) (recenter)))
