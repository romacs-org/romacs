;;; agent-board-roots.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-board-roots :n 200
  :setup (scaling--agent-sessions n (lambda (_) "/tmp/one-project"))
  :body (romacs-agent-board--render it 120))
