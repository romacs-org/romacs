;;; agent-dispatch.el --- scaling probe  -*- lexical-binding: t -*-

;; One event to each of N sessions.
(scaling-probe agent-dispatch :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (mapcar (lambda (id) (let ((s (romacs-agent--make :id id :harness 'claude :title id)))
                                       (puthash id s romacs-agent--sessions) s))
                        (scaling--names "scaling-session" n)))
  :body (let ((record (json-parse-string "{\"seq\": 1, \"out\": {\"type\": \"result\", \"total_cost_usd\": 0}}"
                                         :object-type 'hash-table :null-object :null :false-object :false)))
          (dolist (session it) (romacs-agent--record (romacs-agent-get (romacs-agent-id session)) record))))
