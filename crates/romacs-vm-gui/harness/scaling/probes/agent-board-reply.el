;;; agent-board-reply.el --- scaling probe  -*- lexical-binding: t -*-

;; A board row for a session whose reply is N characters: the row never
;; reads the reply, so its cost does not grow with it.
(scaling-probe agent-board-reply :n 200000 :budget 0.1
  :setup (progn (scaling--agent)
                (require 'romacs-agent-board)
                (let ((session (romacs-agent--make :id "scaling-reply" :harness 'claude :state 'idle :title "reply"
                                                   :reply (make-string n ?a))))
                  (list :kind 'thread :id "scaling-reply" :session session :key 'idle :depth 0)))
  :body (dotimes (_ 200) (romacs-agent-board--line it 120)))
