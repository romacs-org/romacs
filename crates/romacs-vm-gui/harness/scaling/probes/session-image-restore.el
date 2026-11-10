;;; session-image-restore.el --- scaling probe  -*- lexical-binding: t -*-

;; An image of N buffers restored into a session without them.
(scaling-probe session-image-restore :n 200
  :setup (progn (scaling--session n)
                (let ((file (expand-file-name "r.image" warm-restart-directory)))
                  (warm-restart-write nil file)
                  (dolist (name (scaling--names "scaling-session" n)) (kill-buffer name))
                  file))
  :body (warm-restart-restore-image it))
