;;; jabber-mam-redraw-schedule.el --- scaling probe  -*- lexical-binding: t -*-

;; Pages of a catch-up over N peers: each page marks them dirty and
;; schedules their redraw, as `jabber-mam--accept-page' does.
(scaling-probe jabber-mam-redraw-schedule :n 2000
  :setup (progn (setq jabber-mam--dirty-peers nil jabber-mam--redraw-peers nil)
                (cons (scaling--jabber) (scaling--names "peer" n)))
  :body (dotimes (_ 3)
          (dolist (peer (cdr it)) (jabber-mam--mark-dirty (car it) peer "chat"))
          (jabber-mam--schedule-redraw jabber-mam--dirty-peers)
          (when jabber-mam--redraw-timer (cancel-timer jabber-mam--redraw-timer)
                (setq jabber-mam--redraw-timer nil))))
