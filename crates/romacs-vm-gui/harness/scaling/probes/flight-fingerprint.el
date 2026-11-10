;;; flight-fingerprint.el --- scaling probe  -*- lexical-binding: t -*-

;; A flight recorder's step fingerprint (src/flight.rs) over N buffers, one
;; edited before each step: a step looks at each buffer's tick and hashes
;; only the text that changed, not every buffer's.
(scaling-probe flight-fingerprint :n 200
  :setup (let ((buffers (mapcar (lambda (name) (with-current-buffer (get-buffer-create name)
                                                 (insert (make-string 4000 ?x))
                                                 (current-buffer)))
                                (scaling--names "scaling-flight" n))))
           (romacs--flight-fingerprint)
           (vconcat buffers))
  :body (dotimes (i 50)
          (with-current-buffer (aref it (% i (length it))) (insert "y"))
          (romacs--flight-fingerprint)))
