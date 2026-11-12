;;; usage-log-command.el --- scaling probe  -*- lexical-binding: t -*-

;; The usage log's work per command (a caught signal's backtrace, its
;; error record, its slow count) with N kinds already in its slow, traced
;; and captured tables and N timers pending: a fixed 200 commands, so any
;; scan of them shows.
(scaling-probe usage-log-command :n 2000 :budget 0.5
  :setup (progn (require 'romacs-usage-log)
                (define-error 'scaling-usage-error "Scaling usage error")
                (setq romacs-usage-log-file (make-temp-file "scaling-usage-" nil ".jsonl"))
                (dotimes (i n)
                  (puthash (format "command-%d fundamental-mode" i) (vector 1 60 60 0) romacs-usage-log--slow)
                  (puthash (format "command-%d error" i) t romacs-usage-log--traced)
                  (puthash (format "command-%d scaling-usage-error" i) 0 romacs-usage-log--captured))
                ;; Each due before the last, so each is queued at the
                ;; head: timer.el walks the queue past every earlier
                ;; timer, so N of one time are quadratic to queue.
                (dotimes (i n) (run-at-time (+ 3600 (- n i)) nil #'ignore))
                (mapcar #'intern (scaling--names "scaling-usage" 200)))
  :body (let ((signal-hook-function #'romacs-usage-log--signal) (romacs-usage-log-slow-loop-ms 0))
          (setq romacs-usage-log--pending nil)
          (dolist (command it)
            (let ((kind (format "%s scaling-usage-error" command)))
              (remhash kind romacs-usage-log--traced)
              (remhash kind romacs-usage-log--captured))
            (let ((real-this-command command) (this-command command))
              (romacs-usage-log--pre-command)
              (condition-case data (signal 'scaling-usage-error nil)
                (error (romacs-usage-log--command-error data nil nil)))
              (romacs-usage-log--post-command)
              (romacs-usage-log--settled)))))
