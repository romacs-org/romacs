;;; agent-chat-turns.el --- scaling probe  -*- lexical-binding: t -*-

;; A chat of N whole turns: the prompt, a streamed reply, its Markdown, the footer.
(scaling-probe agent-chat-turns :n 100
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-turns-chat" :harness 'claude :title "turns"
                                                   :dir temporary-file-directory)))
                  (romacs-agent-show session)
                  session))
  :body (dotimes (i n)
          (let ((seq (* 10 i)) (id (format "m%d" i)))
            (romacs-agent-ui--render it (list :type 'input :text "a prompt" :seq (+ seq 1) :last t))
            (dotimes (k 4)
              (romacs-agent-ui--render it (list :type 'delta :text "some **reply** text " :message id :seq (+ seq 2 k) :last t)))
            (romacs-agent-ui--render it (list :type 'message-end :message id :seq (+ seq 7) :last t))
            (romacs-agent-ui--render it (list :type 'done :seq (+ seq 8) :last t :duration 1.0)))))
