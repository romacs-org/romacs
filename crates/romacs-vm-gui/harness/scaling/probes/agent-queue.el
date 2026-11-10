;;; agent-queue.el --- scaling probe  -*- lexical-binding: t -*-

;; N messages sent to a per-turn session while its turn runs.
(scaling-probe agent-queue :n 2000
  :setup (progn (scaling--agent)
                (romacs-agent--make :id "scaling-queue" :harness 'codex :title "queue" :state 'running))
  :body (dotimes (i n) (romacs-agent-send it "queued")))
