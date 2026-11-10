;;; inspect-string.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe inspect-string :n 100000 :k 8
  :setup (make-string n ?x)
  :body (dotimes (_ 200) (scaling--inspect-value it)))
