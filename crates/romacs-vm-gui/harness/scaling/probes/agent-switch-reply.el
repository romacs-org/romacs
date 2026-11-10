;;; agent-switch-reply.el --- scaling probe  -*- lexical-binding: t -*-

;; A row with a reply of N characters: it reads what it can show.
(scaling-probe agent-switch-reply :n 200000 :budget 0.1
  :setup (progn (scaling--agent)
                (require 'romacs-agent-ui)
                (romacs-agent--make :id "scaling-reply" :harness 'claude :state 'idle :title "reply"
                                    :reply (make-string n ?a)))
  :body (dotimes (_ 200) (romacs-agent-ui--switch-suffix it 120 nil)))
