;;; agent-dispatch-home.el --- scaling probe  -*- lexical-binding: t -*-

;; One event to each of N sessions with the home page shown: its refresh is
;; scheduled, never rebuilt per event.
(scaling-probe agent-dispatch-home :n 200
  :setup (progn (scaling--agent-sessions n (lambda (i) (format "/tmp/project-%d" (% i 5))))
                (hash-table-values romacs-agent--sessions))
  :body (let ((record (json-parse-string "{\"seq\": 1, \"out\": {\"type\": \"result\", \"total_cost_usd\": 0}}"
                                         :object-type 'hash-table :null-object :null :false-object :false)))
          ;; As if the page were shown (batch has no visible window).
          (cl-letf (((symbol-function 'get-buffer-window) (lambda (&rest _) (selected-window))))
            (dolist (session it) (romacs-agent--record session record)))
          (when (timerp romacs-agents--refresh) (cancel-timer romacs-agents--refresh) (setq romacs-agents--refresh nil))))
