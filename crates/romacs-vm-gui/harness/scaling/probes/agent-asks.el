;;; agent-asks.el --- scaling probe  -*- lexical-binding: t -*-

;; N one-shot questions pending, each session's turn end reaching only its
;; own handler.
(scaling-probe agent-asks :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--asks)
                (mapcar (lambda (id)
                          (let ((session (romacs-agent--make :id id :harness 'claude :title id :state 'running)))
                            (puthash id (lambda (_event) (remhash id romacs-agent--asks)) romacs-agent--asks)
                            session))
                        (scaling--names "scaling-ask" n)))
  :body (progn
          (dolist (session it)
            (romacs-agent--raise session (list :type 'done :text "ok" :seq 1 :last t)))
          (unless (zerop (hash-table-count romacs-agent--asks)) (error "Asks left unanswered"))))
