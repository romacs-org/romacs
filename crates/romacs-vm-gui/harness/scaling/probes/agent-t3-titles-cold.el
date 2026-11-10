;;; agent-t3-titles-cold.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-t3-titles-cold :n 2000
  :setup (scaling--t3-store n)
  :body (progn (clrhash romacs-agent-t3--titles) (romacs-agent-t3-thread-titles it)))
