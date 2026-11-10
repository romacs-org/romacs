;;; session-image-write.el --- scaling probe  -*- lexical-binding: t -*-

;; An image of a session of N buffers written.
(scaling-probe session-image-write :n 200
  :setup (progn (scaling--session n) (expand-file-name "w.image" warm-restart-directory))
  :body (warm-restart-write nil it))
