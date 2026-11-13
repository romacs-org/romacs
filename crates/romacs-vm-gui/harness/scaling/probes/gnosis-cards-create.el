;;; gnosis-cards-create.el --- scaling probe  -*- lexical-binding: t -*-

;; An agent's call creating N Gnosis cards in a collection that already
;; holds N cards agents made: each card is checked against the stored
;; keys by index, not by a scan.
(scaling-probe gnosis-cards-create :n 100
  :setup (progn
           (require 'gnosis-agent-content)
           (setq gnosis-dir (file-name-as-directory (make-temp-file "scaling-gnosis-" t))
                 gnosis-db nil)
           (gnosis--ensure-db)
           (let ((cards (lambda (prefix)
                          (vconcat (mapcar (lambda (i)
                                             (list :type "basic" :question (format "%s question %d" prefix i)
                                                   :answer (vector (format "answer %d" i)) :tags ["scaling"]))
                                           (number-sequence 1 n))))))
             (gnosis-agent-content-create (funcall cards "stored"))
             ;; Warm the creation's code so the body counts only its work.
             (gnosis-agent-content-create (funcall cards "warm") :dry-run t)
             (funcall cards "new")))
  :body (gnosis-agent-content-create it))
