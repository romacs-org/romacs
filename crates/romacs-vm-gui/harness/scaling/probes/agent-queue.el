;;; agent-queue.el --- scaling probe  -*- lexical-binding: t -*-

;; N messages sent to a session while its harness starts: each waits in
;; its queue, joining it in constant time.
(scaling-probe agent-queue :n 2000
  :setup (progn (scaling--agent)
                (romacs-agent--make :id "scaling-queue" :harness 'codex :title "queue" :state 'starting))
  :body (dotimes (i n) (romacs-agent-send it "queued")))
