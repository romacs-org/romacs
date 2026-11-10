;;; agent-images-walk.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-images-walk :n 2000
  :setup (progn (scaling--agent)
                (require 'romacs-agent-images)
                (romacs-agent-images--state (vconcat (scaling--names "scaling-image" n))))
  :body (let ((state it))
          (dotimes (_ (length (plist-get state :files)))
            (setq state (romacs-agent-images--step state ?\C-n)))))
