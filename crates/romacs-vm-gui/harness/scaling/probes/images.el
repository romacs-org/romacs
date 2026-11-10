;;; images.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe images :gui t :n 100 :steps 1
  :setup (progn (clear-image-cache t) (scaling--image-specs n))
  ;; The message is the step's visible change, so the host paints it.
  :body (let (size) (dotimes (_ 2) (dolist (spec it) (setq size (image-size spec t)))) (message "%S" size)))
