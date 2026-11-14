;;; agent-wake-quiet-batch.el --- scaling probe  -*- lexical-binding: t -*-

;; One child's N quiet turn ends (nothing for its parent to act on) handed
;; on in one batch, as a replay hands them: each is let go in one walk.
(scaling-probe agent-wake-quiet-batch :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent-tools--answered)
                (puthash "scaling-parent" (romacs-agent--make :id "scaling-parent" :harness 'codex :title "parent"
                                                              :state 'running)
                         romacs-agent--sessions)
                (clrhash romacs-agent-tools--scheduled)
                (let ((child (romacs-agent--make :id "scaling-turns" :harness 'codex :title "turns"
                                                 :dir (expand-file-name "scaling-turns" (romacs-agent--state-root))
                                                 :options (list :parent "scaling-parent"))))
                  (make-directory (romacs-agent-dir child) t)
                  (puthash "scaling-turns" child romacs-agent--sessions)
                  child))
  :body (progn (dotimes (i n)
                 (romacs-agent--raise it (list :type 'done :text "[quiet] still building" :seq (1+ i) :last t)))
               (romacs-agent-tools--wake it)))
