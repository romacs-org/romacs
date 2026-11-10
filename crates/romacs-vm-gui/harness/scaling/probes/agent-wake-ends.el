;;; agent-wake-ends.el --- scaling probe  -*- lexical-binding: t -*-

;; One child with N turn ends its parent has not had: one wake gives each,
;; in order, behind the parent's running turn.
(scaling-probe agent-wake-ends :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent-tools--answered)
                (puthash "scaling-parent" (romacs-agent--make :id "scaling-parent" :harness 'codex :title "parent"
                                                              :state 'running)
                         romacs-agent--sessions)
                (let ((child (romacs-agent--make :id "scaling-ends" :harness 'codex :title "ends"
                                                 :dir (expand-file-name "scaling-ends" (romacs-agent--state-root))
                                                 :options (list :parent "scaling-parent")
                                                 :ends (let (ends) (dotimes (i n) (push (list (1+ i) "ok" nil) ends)) ends))))
                  (make-directory (romacs-agent-dir child) t)
                  (puthash "scaling-ends" child romacs-agent--sessions)
                  child))
  :body (romacs-agent-tools--wake it))
