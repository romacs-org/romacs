;;; faces-recalc-defface.el --- scaling probe  -*- lexical-binding: t -*-

;; `face-spec-recalc' over N faces on one frame.
(scaling-probe faces-recalc-defface :n 500
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-recalc" n))))
           (dolist (face faces)
             (custom-declare-face face '((((background dark)) :foreground "#aabbcc") (t :inherit bold)) "A scaling face." :group 'scaling))
           faces)
  :body (dolist (face it) (face-spec-recalc face (selected-frame))))
