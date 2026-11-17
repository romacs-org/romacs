;;; agent-wake-parse.el --- scaling probe  -*- lexical-binding: t -*-

;; One delivery of N wakes, with the lines answering each, read back whole:
;; each place is looked at where it is, nothing searched for or reached by
;; counting from the start.  Its results hold non-ASCII text, as real ones do.
(scaling-probe agent-wake-parse :n 400
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "5ca1e000-1111-4222-8333-444455556666" :title "parse")))
                  (concat (mapconcat (lambda (i) (romacs-agent-tools--wake-text
                                                  session (list i (concat (make-string 200 ?r) " — ✓") nil)))
                                     (number-sequence 1 n) "\n\n")
                          "\n\n"
                          (mapconcat (lambda (i) (format "Reply to this result with romacs_task_reply(messageId=\"m-%d\", text=...)." i))
                                     (number-sequence 1 n) "\n"))))
  :body (let ((wakes (romacs-agent-tools--wakes-parse it)))
          ;; Every one of them, or it measures nothing.
          (unless (= (length wakes) n) (error "Parsed %d of %d wakes" (length wakes) n))))
