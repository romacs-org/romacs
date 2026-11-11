;;; agent-tray-far.el --- scaling probe  -*- lexical-binding: t -*-

;; A walk through N images: each key reads the matches filtered once.
;; Keys with the tray's real anchor, hosted, and a composer past a transcript
;; of N lines: what it measures must not grow with what is out of view.
(scaling-probe agent-tray-far :gui t :n 20000 :steps 20 :budget 0.1
  :setup (progn (scaling--agent)
                (require 'romacs-agent-images)
                (let ((buffer (get-buffer-create " *scaling-tray-far*")))
                  (with-current-buffer buffer
                    (erase-buffer)
                    (dotimes (_ n) (insert "a line of the transcript\n"))
                    (setq-local romacs-agent-ui--end (point-max-marker)))
                  (switch-to-buffer buffer)
                  (goto-char (point-min))
                  buffer))
  :body (with-current-buffer it
          (unless (> romacs-agent-ui--end (window-end (selected-window)))
            (error "The composer must be past the window's end"))
          (dotimes (_ 200) (romacs-agent-images--tray-anchor (selected-window) 3))
          ;; A key: each step has something to lay out.
          (insert "x")))
