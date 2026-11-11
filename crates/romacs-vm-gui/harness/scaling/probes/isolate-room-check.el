;;; isolate-room-check.el --- scaling probe  -*- lexical-binding: t -*-

;; The room check at each pooled dispatch (rlisp/vm/isolate.el): one pass
;; over the isolates.  The waiting queue is bounded by `isolate-queue-limit'.
(scaling-probe isolate-room-check :n 2000
  :setup (let ((zygote (record 'isolate-zygote nil nil 'ready nil nil nil nil nil nil 0 nil nil 0 nil))
               isolates)
           (dotimes (i n)
             (push (record 'isolate i nil 'live nil (and (zerop (% i 2)) '((1 . nil))) nil nil nil nil 'job zygote) isolates))
           isolates)
  :body (let ((romacs--isolates it) (isolate-pool-size 1000000)
              (isolate-pool-memory-limit most-positive-fixnum) (isolate-job-memory-headroom 0))
          (dotimes (_ 10) (romacs--isolate-room-p))))
