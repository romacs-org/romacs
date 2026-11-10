;;; jabber-sync-summary.el --- scaling probe  -*- lexical-binding: t -*-

;; A worker's summary of N archives and N changed peers.
(scaling-probe jabber-sync-summary :n 2000
  :setup (let ((archives nil))
           (scaling--jabber)
           (setq jabber-mam-sync--stored (make-hash-table :test #'equal))
           (dolist (room (scaling--names "room" n))
             (push (list :room room :with nil :nick "romeo") archives)
             (puthash (list "romeo@montague.net" room "groupchat") (cons 3 3) jabber-mam-sync--stored))
           (cons (list :username "romeo" :server "montague.net")
                 (cons (list :room nil :with nil :nick nil) archives)))
  :body (jabber-mam-sync--summary (car it) (cdr it) (mapcar (lambda (a) (cons a 'success)) (cdr it)) t 1.0))
