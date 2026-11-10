;;; wrapped-line-continued.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe wrapped-line-continued :gui t :n 1000 :steps 20
  :setup (scaling--wrapped-line n nil)
  :body (with-current-buffer it (scaling--wrapped-step i)))
