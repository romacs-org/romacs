;;; faces-publish-history.el --- scaling probe  -*- lexical-binding: t -*-

;; Typing in one window after N faces were shown in another: what a table
;; remembers must not make every key pay for it.
(scaling-probe faces-publish-history :gui t :n 200 :steps 4
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-history" n)))
               (buffer (get-buffer-create "scaling-history")))
           (dolist (face faces) (custom-declare-face face '((t :foreground "#a0b0c0")) "A scaling face." :group 'scaling))
           (with-current-buffer buffer (dolist (face faces) (insert (propertize "x" 'face face))))
           (switch-to-buffer buffer)
           (redisplay t)
           (let ((typing (get-buffer-create "scaling-typing")))
             (select-window (split-window))
             (switch-to-buffer typing)
             (with-current-buffer buffer (erase-buffer) (insert "x"))
             typing))
  :body (with-current-buffer it (insert "x")))
