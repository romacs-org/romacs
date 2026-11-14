;;; agent-board-filter-chain.el --- scaling probe  -*- lexical-binding: t -*-

;; The agent board filtered to the last of a chain of N delegated threads:
;; every thread above it shows for context, each one's subtree looked at once.
(scaling-probe agent-board-filter-chain :n 50
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent--children)
                (dotimes (i n)
                  (let ((s (romacs-agent--make :id (format "%08d-chain" i) :harness 'claude :provider "claude"
                                               :title (if (= i (1- n)) "the needle" (format "link %d" i))
                                               :cwd "/tmp/chain" :last-time (+ 1e9 i) :state 'idle :turns 1
                                               :options (and (> i 0) (list :parent (format "%08d-chain" (1- i)))))))
                    (puthash (romacs-agent-id s) s romacs-agent--sessions)
                    (romacs-agent--index s)))
                (with-current-buffer (scaling--agent-board)
                  (setq romacs-agent-board--words '("needle"))
                  (current-buffer)))
  :body (romacs-agent-board--render it 120))
