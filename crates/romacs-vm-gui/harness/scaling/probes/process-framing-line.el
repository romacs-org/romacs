;;; process-framing-line.el --- scaling probe  -*- lexical-binding: t -*-

;; One line of N KiB from a child, read a pipe's worth at a time and framed
;; in lines by the process layer (`set-process-framing'): the partial line
;; is held, never copied again as it grows, and the filter is called once,
;; with the whole line.  Held by its slope: instructions per KiB of line,
;; flat.  Measured 2026-11-25 (gate build, batch): 2,400 to 3,300 a KiB
;; over three runs, as the reads a pipe gives vary; the Lisp line framers
;; this replaced took 13,700 a KiB of a 1 MiB line (romacs-agent-rpc.el,
;; read through `cat') and more as the line grew, each read rescanning the
;; partial.  A budget of 6,000 holds the noise and fails a rescan.
(scaling-probe process-framing-line :n 1024 :k 4 :slope 6000
  :setup nil
  :body (let* ((got nil)
               (process (make-process :name "scaling-framing-line"
                                      :command (list "sh" "-c" (format "head -c %d /dev/zero | tr '\\0' x; echo" (* n 1024)))
                                      :connection-type 'pipe :noquery t :coding 'no-conversion
                                      :filter (lambda (_p frames) (setq got (append got frames))))))
          (set-process-framing process 'lines)
          (while (process-live-p process) (accept-process-output process 1))
          (accept-process-output process 0)
          (unless (and (= (length got) 1) (= (length (car got)) (* n 1024)))
            (error "process-framing-line: %d frames, not one of %d KiB" (length got) n))))
