;;; fontset-alternatives.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe fontset-alternatives :n 2000
  :setup nil
  :body (dotimes (_ n) (set-fontset-font t '(#x370 . #x3ff) "X" nil 'prepend)))
