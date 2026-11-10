;;; agent-instances.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-instances :n 1000
  :setup (scaling--t3-instances n)
  :body (let ((romacs-agent-providers it))
          (setq romacs-agent-t3--read nil)
          (romacs-agent-instances t)))
