;;; agent-wake-mixed.el --- scaling probe  -*- lexical-binding: t -*-

;; One child's turns ending one after another behind its parent's running
;; turn, wanted and quiet in turn: the wanted pile up, the quiet go, and
;; neither walks the pile.
(scaling-probe agent-wake-mixed :n 200
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
  :body (dotimes (i n)
          (romacs-agent--raise it (list :type 'done :text (if (cl-oddp i) "[quiet] still" "landed")
                                        :seq (1+ i) :last t))
          (romacs-agent-tools--wake it)))
