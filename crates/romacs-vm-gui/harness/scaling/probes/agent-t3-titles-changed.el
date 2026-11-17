;;; agent-t3-titles-changed.el --- scaling probe  -*- lexical-binding: t -*-

;; The store is changed by its time alone, set back rather than ahead: a
;; file dated in the future reads as just written, and keeps every run
;; that holds it from ever looking idle to what clears old runs away.
(scaling-probe agent-t3-titles-changed :n 2000
  :setup (let ((store (scaling--t3-store n))) (romacs-agent-t3-thread-titles store) store)
  :body (progn (set-file-times it (time-subtract (current-time) (1+ (random 1000000))))
               (romacs-agent-t3-thread-titles it)))
