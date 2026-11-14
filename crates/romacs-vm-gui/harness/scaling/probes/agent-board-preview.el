;;; agent-board-preview.el --- scaling probe  -*- lexical-binding: t -*-

;; The agent board's preview of a thread whose reply is N characters: it
;; shows the reply's head, so its cost does not grow with the reply, but
;; for taking that head.
(scaling-probe agent-board-preview :n 200000 :budget 0.1
  :known "ticket romacs#143: substring costs time linear in the source string, not the part taken"
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (let ((s (romacs-agent--make :id "scaling-preview" :harness 'claude :provider "claude" :title "reply"
                                             :cwd "/tmp/preview" :state 'idle :reply (make-string n ?a))))
                  (puthash (romacs-agent-id s) s romacs-agent--sessions))
                (let ((board (scaling--agent-board)))
                  (romacs-agent-board--render board 120)
                  board))
  :body (with-current-buffer it
          (cl-letf (((symbol-function 'romacs-agent-board--preview-place) (lambda () '(0 0 40 12)))
                    ((symbol-function 'romacs-popup-show) #'ignore))
            (dotimes (_ 20) (romacs-agent-board--preview)))))
