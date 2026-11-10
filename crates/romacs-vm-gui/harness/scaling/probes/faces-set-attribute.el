;;; faces-set-attribute.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe faces-set-attribute :n 200
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces) (custom-declare-face face '((t)) "A scaling face." :group 'scaling))
           faces)
  :body (let ((k 0))
          (dolist (face it)
            (set-face-attribute face nil :foreground (if (cl-oddp k) "red" "blue") :weight 'bold)
            (setq k (1+ k)))))
