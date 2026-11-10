;;; agent-t3-titles-changed.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-t3-titles-changed :n 2000
  :setup (let ((store (scaling--t3-store n))) (romacs-agent-t3-thread-titles store) store)
  :body (progn (set-file-times it (time-add (current-time) (random 1000000)))
               (romacs-agent-t3-thread-titles it)))
