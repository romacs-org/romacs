;;; jabber-mam-dirty-peers.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe jabber-mam-dirty-peers :n 4000
  :setup (progn (setq jabber-mam--dirty-peers nil)
                (cons (scaling--jabber) (scaling--names "peer" (/ n 2))))
  :body (progn
          (dotimes (_ 2)
            (dolist (peer (cdr it)) (jabber-mam--mark-dirty (car it) peer "chat")))))
