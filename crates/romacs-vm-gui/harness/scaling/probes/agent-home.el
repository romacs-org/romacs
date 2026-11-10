;;; agent-home.el --- scaling probe  -*- lexical-binding: t -*-

;; The home page over N sessions, half of them delegated children.
(scaling-probe agent-home :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (dotimes (i n)
                  (let ((s (romacs-agent--make :id (format "%08d-home" i) :harness 'claude :provider "claude"
                                               :title (format "session %d" i) :cwd (format "/tmp/project-%d" (% i 7))
                                               :last-time (+ 1e9 i) :state 'idle :turns 1
                                               :options (and (cl-oddp i) (list :parent (format "%08d-home" (1- i)))))))
                    (puthash (romacs-agent-id s) s romacs-agent--sessions)))
                (with-current-buffer (get-buffer-create "*romacs-agents*") (romacs-agents-mode) (current-buffer)))
  :body (with-current-buffer it (romacs-agent-ui--home)))
