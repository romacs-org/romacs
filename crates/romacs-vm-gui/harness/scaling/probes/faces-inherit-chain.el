;;; faces-inherit-chain.el --- scaling probe  -*- lexical-binding: t -*-

;; One shown face at the end of an inheritance chain of N: realising it walks
;; the chain once, as GNU's merge_named_face does.
(scaling-probe faces-inherit-chain :gui t :n 50 :steps 1
  :setup (let* ((faces (mapcar #'intern (scaling--names "scaling-chain" n)))
                (parent nil))
           (dolist (face faces)
             (custom-declare-face face `((t :inherit ,parent)) "A scaling face." :group 'scaling)
             (setq parent face))
           (let ((buffer (switch-to-buffer (get-buffer-create "scaling-chain"))))
             (insert (propertize "x" 'face parent))
             (cons buffer faces)))
  :body (progn (set-face-attribute (cadr it) nil :foreground "#102030")
               (with-current-buffer (car it) (insert "x"))))
