;;; faces-shared-ancestor.el --- scaling probe  -*- lexical-binding: t -*-

;; N shown faces inheriting one face, which an edit changes: each is
;; realised again, once.
(scaling-probe faces-shared-ancestor :gui t :n 50 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-shared" n))))
           (custom-declare-face 'scaling-shared-parent '((t :foreground "#a0b0c0")) "A scaling face." :group 'scaling)
           (dolist (face faces)
             (custom-declare-face face '((t :inherit scaling-shared-parent)) "A scaling face." :group 'scaling))
           (let ((buffer (switch-to-buffer (get-buffer-create "scaling-shared"))))
             (dolist (face faces) (insert (propertize "x" 'face face)))
             buffer))
  :body (progn (set-face-attribute 'scaling-shared-parent nil :foreground "#102030")
               (with-current-buffer it (insert "x"))))
