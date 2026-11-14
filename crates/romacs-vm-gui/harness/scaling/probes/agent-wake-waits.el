;;; agent-wake-waits.el --- scaling probe  -*- lexical-binding: t -*-

;; Wanted and quiet turn ends in turn behind a busy parent, a wait
;; answering each quiet one after its hand-on let it go: the wait never
;; walks the pile.
(scaling-probe agent-wake-waits :n 200
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
          (romacs-agent-tools--wake it)
          (when (cl-oddp i) (romacs-agent-tools--answer it (1+ i)))))
