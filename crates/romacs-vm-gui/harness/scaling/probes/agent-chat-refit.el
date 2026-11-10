;;; agent-chat-refit.el --- scaling probe  -*- lexical-binding: t -*-

;; A resize of that chat refits its 100 rules: flat in the transcript's size.
(scaling-probe agent-chat-refit :n 100000 :k 10
  :setup (scaling--big-chat n)
  :body (with-current-buffer it
          (dolist (width '(61 62 63 64 65))
            (cl-letf (((symbol-function 'romacs-agent-ui--rule-width) (lambda () width)))
              (romacs-agent-ui--refit-rules)))))
