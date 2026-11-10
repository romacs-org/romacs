;;; tab-bar-faces.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe tab-bar-faces :gui t :n 100 :steps 4
  :setup (scaling--tab-bar n)
  :body (with-current-buffer it (insert "x")))
