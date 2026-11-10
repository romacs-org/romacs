;;; agent-home-projects.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-home-projects :n 200
  :setup (scaling--agent-sessions n (lambda (i) (format "/tmp/project-%d" i)))
  :body (with-current-buffer it (romacs-agent-ui--home)))
