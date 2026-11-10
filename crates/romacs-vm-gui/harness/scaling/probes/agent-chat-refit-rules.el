;;; agent-chat-refit-rules.el --- scaling probe  -*- lexical-binding: t -*-

;; A resize of a chat of N turns refits its N rules once each: linear.
(scaling-probe agent-chat-refit-rules :n 200
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-rules-chat" :harness 'claude :title "rules"
                                                   :dir temporary-file-directory)))
                  (romacs-agent-show session)
                  (dotimes (i n)
                    (romacs-agent-ui--render session (list :type 'input :text "p" :seq (+ (* 10 i) 1) :last t))
                    (romacs-agent-ui--render session (list :type 'done :duration 1.0 :seq (+ (* 10 i) 2) :last t)))
                  (romacs-agent-buffer session)))
  :body (with-current-buffer it
          (dolist (width '(61 62 60 63 59))
            (cl-letf (((symbol-function 'romacs-agent-ui--rule-width) (lambda () width)))
              (romacs-agent-ui--refit-rules)))))
