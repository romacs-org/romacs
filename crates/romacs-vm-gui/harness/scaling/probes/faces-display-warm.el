;;; faces-display-warm.el --- scaling probe  -*- lexical-binding: t -*-

;; A face edit after N faces were realised: what the table held is dropped,
;; and only the one face still shown is realised again.
(scaling-probe faces-display-warm :gui t :n 200 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-warm" n))))
           (dolist (face faces) (custom-declare-face face '((t :foreground "#a0b0c0")) "A scaling face." :group 'scaling))
           (let ((buffer (get-buffer-create "scaling-warm")))
             (with-current-buffer buffer
               (dolist (face faces) (insert (propertize "x" 'face face))))
             (switch-to-buffer buffer)
             (redisplay t)
             (with-current-buffer buffer
               (erase-buffer)
               (insert (propertize "x" 'face (car faces))))
             (redisplay t)
             (cons buffer faces)))
  :body (progn (set-face-attribute (cadr it) nil :foreground "#102030")
               (with-current-buffer (car it) (insert "x"))))
