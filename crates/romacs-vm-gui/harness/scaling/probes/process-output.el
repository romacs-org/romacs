;;; process-output.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe process-output :n 200000
  :setup (get-buffer-create " scaling-process")
  :body (let ((process (make-process :name "scaling" :buffer it :command (list "seq" (number-to-string n))
                                     :connection-type 'pipe :noquery t)))
          (while (process-live-p process) (accept-process-output process 1))
          (accept-process-output process 0)
          (with-current-buffer it (count-lines (point-min) (point-max)))))
