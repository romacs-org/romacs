;;; backtrace-eval-dead-locals.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe backtrace-eval-dead-locals :n 500 :k 4
  :setup (let ((locals (mapcar #'intern (scaling--names "scaling--bt-local" n)))
               (globals (mapcar #'intern (scaling--names "scaling--bt-global" n))))
           (dolist (g globals) (set g 0))
           (list locals globals))
  :body (with-temp-buffer
          (dotimes (_ 4)
            (dolist (l (car it)) (set (make-local-variable l) 0))
            (scaling--bt-dead-outer (car it) (cadr it)))))
