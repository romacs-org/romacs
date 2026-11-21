;;; agent-chat-big-output.el --- scaling probe  -*- lexical-binding: t -*-

;; A command whose output is N lines ends: its row and its last lines are
;; drawn, the output itself kept aside.  What is drawn is a few lines of
;; the window's width, whatever N: the draw must not grow with N.
(scaling-probe agent-chat-big-output :n 50000 :k 4 :budget 0.25
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-big" :harness 'claude :title "scaling big"
                                                   :dir temporary-file-directory))
                      (input (make-hash-table :test #'equal)))
                  (puthash "command" "make" input)
                  (romacs-agent-show session)
                  (list session input
                        (concat
                         (mapconcat (lambda (i) (format "line %d of a long build log, with some words" i))
                                    (number-sequence 1 n) "\n")
                         ;; The note Claude Code puts after a command's output.
                         "\nShell cwd was reset to /tmp\n"))))
  :body (pcase-let ((`(,session ,input ,output) it))
          (romacs-agent-log--on-event session (list :type 'tool :state 'started :id "big" :name "Bash" :input input
                                                    :seq 1 :last t))
          (romacs-agent-log--on-event session (list :type 'tool :state 'completed :id "big" :output output
                                                    :seq 2 :last t))))
