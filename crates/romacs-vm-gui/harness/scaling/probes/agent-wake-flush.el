;;; agent-wake-flush.el --- scaling probe  -*- lexical-binding: t -*-

;; N children each end a turn, then one flush hands every end on to their
;; idle parent: each end joins the parent's waiting ends in constant time
;; (the wake itself is a timer, which the probe never runs), and the
;; flush reads them once.
(scaling-probe agent-wake-flush :n 400
  :setup (progn (scaling--agent)
                (require 'romacs-agent-tools)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent-tools--answered)
                (clrhash romacs-agent-tools--scheduled)
                (clrhash romacs-agent-tools--pending)
                (let ((parent (romacs-agent--make :id "scaling-parent" :harness 'codex :title "parent"
                                                  :state 'idle)))
                  (puthash "scaling-parent" parent romacs-agent--sessions)
                  (cons parent
                        (cl-loop for i below n
                                 collect (let* ((id (format "scaling-child-%d" i))
                                                (child (romacs-agent--make
                                                        :id id :harness 'codex :title id
                                                        :dir (expand-file-name id (romacs-agent--state-root))
                                                        :options (list :parent "scaling-parent")
                                                        ;; One turn ended, as `romacs-agent--note-end' keeps it.
                                                        :ends (list (list 1 "ok" nil)))))
                                           (make-directory (romacs-agent-dir child) t)
                                           (puthash id child romacs-agent--sessions)
                                           child)))))
  :body (progn
          (dolist (child (cdr it))
            (romacs-agent-tools--wake child))
          (cl-letf (((symbol-function 'romacs-agent-tools--deliver) #'ignore))
            (romacs-agent-tools--flush (car it)))))
