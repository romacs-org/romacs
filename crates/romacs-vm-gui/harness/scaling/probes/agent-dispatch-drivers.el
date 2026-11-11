;;; agent-dispatch-drivers.el --- scaling probe  -*- lexical-binding: t -*-

;; The same with 50 more harness drivers defined ahead of Claude Code's: a
;; record reaches its session's driver through the session, never a
;; registry lookup.
(scaling-probe agent-dispatch-drivers :n 200
  :setup (progn (scaling--agent)
                (setq romacs-agent--drivers
                      (append (cl-loop for i below 50
                                       collect (list (intern (format "scaling-driver-%d" i))
                                                     :lifetime 'persistent
                                                     :config-variable (format "SCALING_DRIVER_%d_HOME" i)))
                              (cl-remove-if (lambda (d) (string-prefix-p "scaling-driver-" (symbol-name (car d))))
                                            romacs-agent--drivers)))
                (clrhash romacs-agent--sessions)
                (mapcar (lambda (id) (let ((s (romacs-agent--make :id id :harness 'claude :title id)))
                                       (puthash id s romacs-agent--sessions) s))
                        (scaling--names "scaling-session" n)))
  :body (let ((record (json-parse-string "{\"seq\": 1, \"out\": {\"type\": \"result\", \"total_cost_usd\": 0}}"
                                         :object-type 'hash-table :null-object :null :false-object :false)))
          (dolist (session it) (romacs-agent--record (romacs-agent-get (romacs-agent-id session)) record))))
