;;; agent-wake-parse.el --- scaling probe  -*- lexical-binding: t -*-

;; One delivery of N wakes, with the lines answering each, read back: each
;; place is looked at where it is, nothing searched for or copied twice.
(scaling-probe agent-wake-parse :n 400
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-parse" :title "parse")))
                  (concat (mapconcat (lambda (i) (romacs-agent-tools--wake-text session (list i (make-string 200 ?r) nil)))
                                     (number-sequence 1 n) "\n\n")
                          "\n\n"
                          (mapconcat (lambda (i) (format "Reply to this result with romacs_task_reply(messageId=\"m-%d\", text=...)." i))
                                     (number-sequence 1 n) "\n"))))
  :body (romacs-agent-tools--wakes-parse it))
