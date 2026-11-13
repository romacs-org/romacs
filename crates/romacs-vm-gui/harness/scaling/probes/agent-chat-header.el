;;; agent-chat-header.el --- scaling probe  -*- lexical-binding: t -*-

;; A chat's header kept up with its session's events in a chat of N
;; characters: each refresh reads the session, never the transcript, and
;; only every tenth event here changes what the header says: flat.
(scaling-probe agent-chat-header :n 100000 :k 10
  :setup (scaling--big-chat n)
  :body (let ((session (buffer-local-value 'romacs-agent-ui--session it)))
          (romacs-agent--raise session (list :type 'done :context-window 200000))
          (dotimes (i 200)
            (romacs-agent--raise session (list :type 'usage :context-used (* 10000 (/ i 10)))))))
