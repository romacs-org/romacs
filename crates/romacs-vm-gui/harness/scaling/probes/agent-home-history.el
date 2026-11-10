;;; agent-home-history.el --- scaling probe  -*- lexical-binding: t -*-

;; The home page over N transcripts beside a session per project.
(scaling-probe agent-home-history :n 200
  :setup (let ((dir (scaling--history n)))
           (scaling--with-history-place dir (romacs-agent-history-list))
           (clrhash romacs-agent--sessions)
           (with-current-buffer (get-buffer-create "*romacs-agents*") (romacs-agents-mode) (current-buffer)))
  :body (with-current-buffer it (romacs-agent-ui--home)))
