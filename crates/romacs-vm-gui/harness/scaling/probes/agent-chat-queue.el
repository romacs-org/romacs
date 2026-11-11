;;; agent-chat-queue.el --- scaling probe  -*- lexical-binding: t -*-

;; Composer updates with N messages queued: the count is kept, not taken.
(scaling-probe agent-chat-queue :n 2000
  :setup (let ((session (progn (scaling--agent) (romacs-agent--make :id "scaling-queue-chat" :harness 'claude :title "queue"
                                            :dir temporary-file-directory :state 'running))))
           (romacs-agent-show session)
           (dotimes (i n) (romacs-agent-send-later session (format "message %d" i)))
           (romacs-agent-buffer session))
  :body (with-current-buffer it (dotimes (_ 200) (romacs-agent-ui--composer-update))))
