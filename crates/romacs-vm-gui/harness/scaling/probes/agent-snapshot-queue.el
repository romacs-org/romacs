;;; agent-snapshot-queue.el --- scaling probe  -*- lexical-binding: t -*-

;; A session's project snapshots queue behind the one running: N join the
;; queue, each in constant time, then leave it one by one, oldest first.
;; Taking a snapshot is a stand-in that records the turn, so the probe
;; measures the queue, not git.
(scaling-probe agent-snapshot-queue :n 4000
  :setup (progn (scaling--agent)
                (require 'romacs-agent-diff)
                (clrhash romacs-agent-diff--queues)
                (romacs-agent--make :id "scaling-snapshots" :harness 'codex :title "snapshots"))
  :body (let ((ran nil))
          (cl-letf (((symbol-function 'romacs-agent-diff--run)
                     (lambda (_session turn _edge _then) (push turn ran))))
            ;; The first runs at once; the rest wait behind it.
            (dotimes (i n) (romacs-agent-diff--snapshot it i 'start))
            (dotimes (_ n) (romacs-agent-diff--next it)))
          (unless (equal (nreverse ran) (number-sequence 0 (1- n)))
            (error "Snapshots ran out of order"))))
