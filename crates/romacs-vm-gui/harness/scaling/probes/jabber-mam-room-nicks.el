;;; jabber-mam-room-nicks.el --- scaling probe  -*- lexical-binding: t -*-

;; Our nickname looked up once per archived result, over N rooms read unjoined.
(scaling-probe jabber-mam-room-nicks :n 4000
  :setup (let ((nicks (make-hash-table :test #'equal)) (rooms (scaling--names "room" n)))
           (scaling--jabber)
           (dolist (room rooms) (puthash room "romeo" nicks))
           (setq jabber-mam--room-nicks nicks)
           (cons (car jabber-connections) rooms))
  :body (dolist (room (cdr it)) (jabber-mam--our-muc-nick-p room "romeo" (car it))))
