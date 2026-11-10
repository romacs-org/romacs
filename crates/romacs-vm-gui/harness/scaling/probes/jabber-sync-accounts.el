;;; jabber-sync-accounts.el --- scaling probe  -*- lexical-binding: t -*-

;; N accounts, each requesting its personal archive.
(scaling-probe jabber-sync-accounts :n 400
  :setup (progn
           (scaling--jabber)
           (clrhash jabber-mam-sync--accounts)
           (mapcar (lambda (name)
                     (let ((jc (make-symbol name)))
                       (put jc :state :session-established)
                       (put jc :state-data (list :username name :server "montague.net" :resource "r"))
                       jc))
                   (scaling--names "user" n)))
  :body (dolist (jc it) (jabber-mam-sync-request jc)))
