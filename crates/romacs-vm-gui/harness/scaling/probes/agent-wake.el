;;; agent-wake.el --- scaling probe  -*- lexical-binding: t -*-

;; N children of one parent each ending a turn while the parent's turn
;; runs: each wake is queued behind it once (the real send and queue).
(scaling-probe agent-wake :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent-tools--answered)
                (let ((parent (romacs-agent--make :id "scaling-parent" :harness 'codex :title "parent"
                                                  :state 'running)))
                  (puthash "scaling-parent" parent romacs-agent--sessions))
                (mapcar (lambda (id)
                          (let ((child (romacs-agent--make :id id :harness 'codex :title id :done-seq 1
                                                           :ends (list (list 1 "ok" nil))
                                                           :dir (expand-file-name id (romacs-agent--state-root))
                                                           :options (list :parent "scaling-parent"))))
                            (make-directory (romacs-agent-dir child) t)
                            (puthash id child romacs-agent--sessions)
                            child))
                        (scaling--names "scaling-child" n)))
  :body (dolist (child it) (romacs-agent-tools--wake child)))
