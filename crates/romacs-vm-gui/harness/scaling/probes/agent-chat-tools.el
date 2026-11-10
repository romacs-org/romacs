;;; agent-chat-tools.el --- scaling probe  -*- lexical-binding: t -*-

;; A chat of N tool calls, started then each answered.
(scaling-probe agent-chat-tools :n 200
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-chat" :harness 'claude :title "scaling chat"
                                                   :dir temporary-file-directory)))
                  (romacs-agent-show session)
                  session))
  :body (progn
          (dotimes (i n)
            (romacs-agent-ui--render it (list :type 'tool :state 'started :id (format "t%d" i) :name "Bash"
                                              :seq (1+ i) :last t)))
          (dotimes (i n)
            (romacs-agent-ui--render it (list :type 'tool :state 'completed :id (format "t%d" i) :output "ok"
                                              :seq (+ 1 n i) :last t)))))
