;;; wrapped-line-words.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe wrapped-line-words :gui t :n 1000 :steps 20
  :setup (scaling--wrapped-line n t)
  :body (with-current-buffer it (scaling--wrapped-step i)))
