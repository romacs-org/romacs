;;; inspect-stack.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe inspect-stack :n 1000 :k 4
  :setup (setq max-lisp-eval-depth (max max-lisp-eval-depth (* 20 n)))
  :body (dotimes (_ 100) (scaling--inspect-deep n)))
