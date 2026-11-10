;;; agent-history-relist.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-history-relist :n 200
  :setup (let ((dir (scaling--history n))) (scaling--with-history-place dir (romacs-agent-history-list)) dir)
  :body (scaling--with-history-place it (romacs-agent-history-list)))
