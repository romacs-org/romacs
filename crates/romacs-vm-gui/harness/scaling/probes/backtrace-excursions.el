;;; backtrace-excursions.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe backtrace-excursions :n 200 :k 4
  :setup (setq max-lisp-eval-depth (max max-lisp-eval-depth (* 20 n)))
  :body (dotimes (_ 4) (scaling--bt-excursions n n)))
