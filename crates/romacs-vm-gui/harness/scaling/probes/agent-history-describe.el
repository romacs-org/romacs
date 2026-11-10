;;; agent-history-describe.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-history-describe :n 100
  :setup (scaling--history n)
  :body (scaling--with-history-place it
          (clrhash romacs-agent-history--descriptions)
          (romacs-agent-history-list)))
