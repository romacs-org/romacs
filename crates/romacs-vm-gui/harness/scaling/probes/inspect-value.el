;;; inspect-value.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe inspect-value :n 20000 :k 8
  :setup (make-list n 'x)
  :body (dotimes (_ 200) (scaling--inspect-value it)))
