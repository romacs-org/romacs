;;; agent-instances-kept.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-instances-kept :n 1000
  ;; Read once here: the body resolves with the file's read kept.
  :setup (let ((providers (scaling--t3-instances n)))
           (let ((romacs-agent-providers providers)) (romacs-agent-instances t))
           providers)
  :body (let ((romacs-agent-providers it)) (romacs-agent-instances t)))
