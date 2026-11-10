;;; tab-bar-faces-made.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe tab-bar-faces-made :gui t :n 100 :steps 4
  :setup (scaling--tab-bar n)
  :body (with-current-buffer it
          (set-face-attribute 'highlight nil :weight (if (= (% i 2) 0) 'bold 'normal))
          (insert "x")))
