;;; agent-board-projects.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-board-projects :n 200
  :setup (scaling--agent-sessions n (lambda (i) (format "/tmp/project-%d" i)))
  :body (romacs-agent-board--render it 120))
