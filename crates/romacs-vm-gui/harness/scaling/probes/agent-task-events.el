;;; agent-task-events.el --- scaling probe  -*- lexical-binding: t -*-

;; A harness runs N commands as tasks, one after another, and names each
;; only by its task id once started: each event finds its row by that id,
;; however many finished rows the session keeps.
(scaling-probe agent-task-events :n 2000
  :setup (progn (scaling--agent)
                (require 'romacs-agent-ui)
                (romacs-agent--make :id "scaling-tasks" :harness 'claude :title "tasks" :state 'running))
  :body (dotimes (i n)
          (let ((task (format "task-%d" i)))
            (romacs-agent-ui--track-task it (list :state 'started :id (format "tool-%d" i) :task task))
            (romacs-agent-ui--track-task it (list :state 'progress :task task :activity "working"))
            (romacs-agent-ui--track-task it (list :state 'completed :task task)))))
