;;; agent-home-roots.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-home-roots :n 200
  :setup (scaling--agent-sessions n (lambda (_) "/tmp/one-project"))
  :body (with-current-buffer it (romacs-agent-ui--home)))
