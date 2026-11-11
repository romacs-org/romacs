;;; redisplay-buffers.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe redisplay-buffers :gui t :n 100 :steps 20 :budget 0.5
  :setup (progn
           (dolist (name (scaling--names "scaling-chrome" n))
             (with-current-buffer (get-buffer-create name) (insert name)))
           (switch-to-buffer (get-buffer-create "scaling-chrome-shown"))
           (current-buffer))
  :body (with-current-buffer it (insert "x")))
