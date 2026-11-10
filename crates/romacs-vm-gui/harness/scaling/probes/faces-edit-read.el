;;; faces-edit-read.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe faces-edit-read :gui t :n 100 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces) (custom-declare-face face '((t)) "A scaling face." :group 'scaling))
           (cons (switch-to-buffer (get-buffer-create "scaling-edit")) faces))
  ;; None of the faces shows, so the step paints by typing.
  :body (progn (dolist (face (cdr it))
                 (set-face-attribute face nil :foreground "red")
                 (face-attribute face :foreground))
               (with-current-buffer (car it) (insert "x"))))
