;;; agent-switch-reply.el --- scaling probe  -*- lexical-binding: t -*-

;; A switcher row for a session whose reply is N characters: the row never
;; reads the reply, so its cost does not grow with it.
(scaling-probe agent-switch-reply :n 200000 :budget 0.1
  :setup (progn (scaling--agent)
                (require 'romacs-agent-ui)
                (romacs-agent--make :id "scaling-reply" :harness 'claude :state 'idle :title "reply"
                                    :reply (make-string n ?a)))
  :body (dotimes (_ 200) (romacs-agent-ui--switch-suffix it 120)))
