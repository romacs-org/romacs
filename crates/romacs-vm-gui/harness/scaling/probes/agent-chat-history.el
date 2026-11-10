;;; agent-chat-history.el --- scaling probe  -*- lexical-binding: t -*-

;; M-p through a history of N prompts, then M-n back: each step one cell.
(scaling-probe agent-chat-history :n 2000
  :setup (let ((session (progn (scaling--agent) (romacs-agent--make :id "scaling-history-chat" :harness 'claude :title "history"
                                            :dir temporary-file-directory))))
           (romacs-agent-show session)
           (with-current-buffer (romacs-agent-buffer session)
             (setq romacs-agent-ui--history (mapcar #'list (scaling--names "prompt" n)))
             (goto-char (point-max))
             (current-buffer)))
  :body (with-current-buffer it
          (dotimes (_ n) (romacs-agent-chat-history-previous))
          (dotimes (_ n) (romacs-agent-chat-history-next))))
