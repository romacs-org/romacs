;;; agent-gnosis-tokens.el --- scaling probe  -*- lexical-binding: t -*-

;; N gnosis cursors and plans handed out as tokens (the newest 256 kept),
;; each looked up again.
(scaling-probe agent-gnosis-tokens :n 400
  :setup (progn (scaling--agent) (require 'romacs-agent-gnosis)
                (clrhash romacs-agent-gnosis--tokens) (fillarray romacs-agent-gnosis--ring nil)
                (number-sequence 1 n))
  :body (dolist (i it)
          (romacs-agent-gnosis--value-of (romacs-agent-gnosis--token (list :after i)))))
