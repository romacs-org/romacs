;;; faces-defface.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe faces-defface :n 200
  :known "ticket faces-defface: custom-add-to-group is GNU's member + nconc over the whole group"
  :setup (mapcar #'intern (scaling--names "scaling-face" n))
  :body (dolist (face it) (custom-declare-face face '((t :weight bold)) "A scaling face." :group 'scaling)))
