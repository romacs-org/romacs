;;; backtrace-frames.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe backtrace-frames :n 200 :k 4
  :setup (progn (require 'backtrace) (require 'romacs-backtrace)
                (setq max-lisp-eval-depth (max max-lisp-eval-depth (* 20 n))))
  :body (dotimes (_ 4) (scaling--bt-frames n)))
