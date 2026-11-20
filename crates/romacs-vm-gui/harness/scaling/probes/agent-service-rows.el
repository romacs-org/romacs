;;; agent-service-rows.el --- scaling probe  -*- lexical-binding: t -*-

;; The agents service's list of N sessions, half of them delegated: ten
;; sessions' events remake their rows, their parents' and the index, a
;; cost that N does not change (romacs-agent-service.el).  Measured at a
;; slope of 6.5 instructions a unit of N (the tables' own growth); 50
;; leaves room for that, and none for a pass over the rows.
(scaling-probe agent-service-rows :n 400 :slope 50
  :setup (progn (scaling--agent) (require 'romacs-agent-service)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent--children)
                (dotimes (i n)
                  (let ((s (romacs-agent--make :id (format "%08d-service" i) :harness 'claude :provider "claude"
                                               :title (format "session %d" i) :cwd (format "/tmp/project-%d" (% i 7))
                                               :last-time (+ 1e9 i) :state 'idle :turns 1
                                               :options (and (cl-oddp i) (list :parent (format "%08d-service" (1- i)))))))
                    (puthash (romacs-agent-id s) s romacs-agent--sessions)
                    (romacs-agent--index s)))
                (let ((list (romacs-agent-service--list-make)))
                  (romacs-agent-service--apply list (romacs-agent-service--whole-updates list))
                  list))
  :body (romacs-agent-service--apply it (romacs-agent-service--rows-of
                                         it (mapcar (lambda (i) (format "%08d-service" i)) (number-sequence 1 19 2)))))
