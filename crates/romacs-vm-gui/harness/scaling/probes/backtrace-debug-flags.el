;;; backtrace-debug-flags.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe backtrace-debug-flags :n 400 :k 4
  :setup (setq max-lisp-eval-depth (max max-lisp-eval-depth (* 20 n)))
  :body (let ((debugger (lambda (&rest args) (cadr args))))
          (scaling--bt-flag-deep n)
          (scaling--bt-flag-any n)
          (dotimes (_ n) (scaling--bt-flag-leave))))
