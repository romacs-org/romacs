;;; agent-board-dispatch.el --- scaling probe  -*- lexical-binding: t -*-

;; One event to each of N sessions with the agent board shown: drawn again
;; once for the batch, never per event.
(scaling-probe agent-board-dispatch :n 200
  :setup (progn (scaling--agent-sessions n (lambda (i) (format "/tmp/project-%d" (% i 5))))
                (require 'romacs-agent-ui)
                (hash-table-values romacs-agent--sessions))
  :body (let ((record (json-parse-string "{\"seq\": 1, \"out\": {\"type\": \"result\", \"total_cost_usd\": 0}}"
                                         :object-type 'hash-table :null-object :null :false-object :false)))
          (dolist (session it) (romacs-agent--record session record))
          ;; As if the board were shown (batch has no visible window).
          (cl-letf (((symbol-function 'romacs-agent-board--shown) #'romacs-agent-board--buffer)
                    ((symbol-function 'romacs-popup-show) #'ignore))
            (when (timerp romacs-agent-ui--children-timer) (cancel-timer romacs-agent-ui--children-timer))
            (romacs-agent-ui--children-flush))))
