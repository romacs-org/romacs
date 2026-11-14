;;; agent-board.el --- scaling probe  -*- lexical-binding: t -*-

;; The agent board over N sessions, half of them delegated children.
(scaling-probe agent-board :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent--children)
                (dotimes (i n)
                  (let ((s (romacs-agent--make :id (format "%08d-board" i) :harness 'claude :provider "claude"
                                               :title (format "session %d" i) :cwd (format "/tmp/project-%d" (% i 7))
                                               :last-time (+ 1e9 i) :state 'idle :turns 1
                                               :options (and (cl-oddp i) (list :parent (format "%08d-board" (1- i)))))))
                    (puthash (romacs-agent-id s) s romacs-agent--sessions)
                    (romacs-agent--index s)))
                (scaling--agent-board))
  :body (romacs-agent-board--render it 120))
