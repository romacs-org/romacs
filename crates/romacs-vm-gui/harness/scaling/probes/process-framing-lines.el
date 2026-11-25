;;; process-framing-lines.el --- scaling probe  -*- lexical-binding: t -*-

;; N lines of a child's output (`seq'), framed in lines by the process
;; layer (`set-process-framing'): its filter is called once a read with the
;; lines the read completed.  Each byte is scanned once, so the work is
;; linear in N, whatever lines a read holds.
(scaling-probe process-framing-lines :n 100000
  :setup nil
  :body (let* ((lines 0)
               (process (make-process :name "scaling-framing-lines" :command (list "seq" (number-to-string n))
                                      :connection-type 'pipe :noquery t :coding 'no-conversion
                                      :filter (lambda (_p frames) (setq lines (+ lines (length frames)))))))
          (set-process-framing process 'lines)
          (while (process-live-p process) (accept-process-output process 1))
          (accept-process-output process 0)
          (unless (= lines n) (error "process-framing-lines: %d lines, not %d" lines n))))
