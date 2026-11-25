;;; agent-board-reply.el --- scaling probe  -*- lexical-binding: t -*-

;; A board row for a session whose reply is N characters: the row never
;; reads the reply, so its cost does not grow with it.  The row is the
;; board's own, made from the session's summary.
(scaling-probe agent-board-reply :n 200000 :budget 0.1
  :setup (progn (scaling--agent)
                (require 'romacs-agent-board)
                (clrhash romacs-agent--sessions)
                (let ((session (romacs-agent--make :id "scaling-reply" :harness 'claude :state 'idle :title "reply"
                                                   :cwd "/tmp/reply" :reply (make-string n ?a))))
                  (puthash (romacs-agent-id session) session romacs-agent--sessions)
                  (seq-find (lambda (row) (eq (plist-get row :kind) 'thread))
                            (romacs-agent-board-rows (list (romacs-agent-summary "scaling-reply")) nil
                                                     (list :folds (make-hash-table :test #'equal))))))
  :body (dotimes (_ 200) (romacs-agent-board--line it 120)))
