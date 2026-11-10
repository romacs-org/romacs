;;; usage-log-prune.el --- scaling probe  -*- lexical-binding: t -*-

;; A server's start prunes the session records: N kept because their
;; processes live (pid 1), each looked at once.
(scaling-probe usage-log-prune :n 500
  :setup (let ((root (make-temp-file "scaling-sessions-" t)))
           (require 'romacs-usage-log)
           (dotimes (i n) (make-directory (expand-file-name (format "20200101T%06d-1" i) root)))
           root)
  :body (romacs-usage-log--prune it 50))
