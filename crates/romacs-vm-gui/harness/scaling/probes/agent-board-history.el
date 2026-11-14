;;; agent-board-history.el --- scaling probe  -*- lexical-binding: t -*-

;; The agent board over N transcripts, every one shown.
(scaling-probe agent-board-history :n 200
  :setup (let ((dir (scaling--history n)))
           (scaling--with-history-place dir (romacs-agent-history-list))
           (clrhash romacs-agent--sessions)
           (with-current-buffer (scaling--agent-board)
             (setq romacs-agent-board--all-history t)
             (current-buffer)))
  :body (romacs-agent-board--render it 120))
