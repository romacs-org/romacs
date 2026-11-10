;;; jabber-sync-requests.el --- scaling probe  -*- lexical-binding: t -*-

;; N rooms requested for one account's background catch-up.
(scaling-probe jabber-sync-requests :n 400
  :setup (let ((jc (scaling--jabber))) (clrhash jabber-mam-sync--accounts) (cons jc (scaling--names "room" n)))
  :body (dolist (room (cdr it)) (jabber-mam-sync-request (car it) room)))
