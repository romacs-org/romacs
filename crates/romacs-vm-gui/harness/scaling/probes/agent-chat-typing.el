;;; agent-chat-typing.el --- scaling probe  -*- lexical-binding: t -*-

;; Keys typed into that chat's composer (with the post-command hook): flat.
(scaling-probe agent-chat-typing :n 100000 :k 10
  :setup (with-current-buffer (scaling--big-chat n) (goto-char (point-max)) (current-buffer))
  :body (with-current-buffer it
          (dotimes (_ 50)
            (let ((last-command-event ?a) (this-command 'self-insert-command))
              (call-interactively #'self-insert-command)
              (run-hooks 'post-command-hook)))
          (dotimes (_ 10)
            (let ((this-command 'backward-kill-word))
              (call-interactively #'backward-kill-word)
              (run-hooks 'post-command-hook)))))
