;;; flight-buffer-steps.el --- scaling probe  -*- lexical-binding: t -*-

;; A session that makes a buffer at each step: the steps' fingerprints
;; look at every buffer's tick, a scan of the buffer list per step.
(scaling-probe flight-buffer-steps :n 200
  :known "ticket flight-steps: a flight step looks at each buffer's tick"
  :setup (scaling--names "scaling-flight-steps" n)
  :body (dolist (name it)
          (with-current-buffer (get-buffer-create name) (insert "x"))
          (romacs--flight-fingerprint)))
