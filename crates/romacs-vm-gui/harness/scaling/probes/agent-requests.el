;;; agent-requests.el --- scaling probe  -*- lexical-binding: t -*-

;; N delegated tasks, each indexed by its client request id and found again.
(scaling-probe agent-requests :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--requests)
                (mapcar (lambda (id) (romacs-agent--make :id id :harness 'codex :title id
                                                        :options (list :parent "scaling-parent" :request id)))
                        (scaling--names "scaling-request" n)))
  :body (progn
          (dolist (session it)
            (puthash (romacs-agent-id session) session romacs-agent--sessions)
            (romacs-agent--index session))
          (dolist (session it) (romacs-agent-requested "scaling-parent" (romacs-agent-id session)))))
