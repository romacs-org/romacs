;;; agent-watches.el --- scaling probe  -*- lexical-binding: t -*-

;; A tick of the unit reactor over N watched units: systemd's answer for
;; all of them read once, and the ended ones found, in one pass each.
(scaling-probe agent-watches :n 1000
  :setup (progn (scaling--agent) (require 'romacs-agent-watch)
                (cons (cl-loop for i below n collect (cons (format "task-%d" i) (list :unit (format "check-%d" i))))
                      (mapconcat (lambda (i) (format "Id=check-%d.service\nActiveState=%s\n" i
                                                     (if (cl-evenp i) "active" "inactive")))
                                 (number-sequence 0 (1- n)) "\n")))
  :body (romacs-agent-watch--ended (car it) (romacs-agent-watch--states (cdr it))))
