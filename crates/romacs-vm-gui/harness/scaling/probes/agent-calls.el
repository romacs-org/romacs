;;; agent-calls.el --- scaling probe  -*- lexical-binding: t -*-

;; N calls recorded in a session's call log (each an appended line) and
;; each looked up.
(scaling-probe agent-calls :n 200
  :setup (progn (scaling--agent)
                (make-directory (romacs-agent-tools--dir "scaling-calls") t)
                (scaling--names "relay:call" n))
  :body (progn
          (dolist (key it) (romacs-agent-tools--note "scaling-calls" key (romacs-mcp--object "state" "done")))
          (dolist (key it) (gethash key (romacs-agent-tools--table "scaling-calls")))))
