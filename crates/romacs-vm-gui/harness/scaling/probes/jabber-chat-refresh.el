;;; jabber-chat-refresh.el --- scaling probe  -*- lexical-binding: t -*-

;; A room buffer refreshed over N stored messages (`jabber-backlog-number'
;; N): what runs before the first idle tick, the database's materialization
;; of the backlog among it.  The clearing and insertion ticks run later.
(scaling-probe jabber-chat-refresh :n 300
  :setup (let* ((jc (scaling--jabber))
                (room "room@conference.montague.net"))
           (setq jabber-db-path (make-temp-file "scaling-jabber" nil ".db")
                 jabber-db--connection nil
                 jabber-backlog-number n)
           (dotimes (i n)
             (jabber-db-store-message "romeo@montague.net" room "in" "groupchat"
                                      (format "message %d %s" i (make-string 60 ?x))
                                      (+ 1700000000 i) "nick" (format "s%d" i)))
           (let ((buffer (jabber-muc-create-buffer jc room)))
             ;; The creation's own backlog load: let it finish first.
             (with-current-buffer buffer
               (cl-incf jabber-chat--backlog-generation))
             buffer))
  :body (with-current-buffer it (jabber-chat-buffer-refresh)))
