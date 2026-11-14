;;; gnosis-cards-retry.el --- scaling probe  -*- lexical-binding: t -*-

;; An agent's retried call of N Gnosis cards it already made: each is
;; found by its stored key and reported, none made again.
(scaling-probe gnosis-cards-retry :n 100
  :setup (progn
           (require 'gnosis-agent-content)
           (setq gnosis-dir (file-name-as-directory (make-temp-file "scaling-gnosis-" t))
                 gnosis-db nil)
           (gnosis--ensure-db)
           (let ((cards (vconcat (mapcar (lambda (i)
                                           (list :type "basic" :question (format "question %d" i)
                                                 :answer (vector (format "answer %d" i))))
                                         (number-sequence 1 n)))))
             (gnosis-agent-content-create cards)
             ;; Warm the retry's code so the body counts only its work.
             (gnosis-agent-content-create cards)
             cards))
  :body (gnosis-agent-content-create it))
