;;; obarray-intern.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe obarray-intern :n 20000
  :setup (scaling--names "scaling-symbol" n)
  :body (progn (dolist (name it) (intern name)) (dolist (name it) (intern-soft name))))
