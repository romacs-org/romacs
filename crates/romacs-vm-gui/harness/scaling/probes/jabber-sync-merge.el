;;; jabber-sync-merge.el --- scaling probe  -*- lexical-binding: t -*-

;; A finished job's N changed peers against N open (hidden) chat buffers:
;; the histories are marked changed; no buffer is looked for.
(scaling-probe jabber-sync-merge :n 400
  :setup (let ((jc (scaling--jabber)) peers)
           (setq jabber-mam-sync--merge-queue nil)
           (dolist (room (scaling--names "room" n))
             (with-current-buffer (get-buffer-create (concat " *scaling-" room "*"))
               (setq major-mode 'jabber-chat-mode)
               (setq-local jabber-buffer-connection jc)
               (setq-local jabber-group room)
               (jabber-buffer-registry-register 'muc (list "romeo@montague.net" room)))
             (push (list "romeo@montague.net" room "groupchat" 1 1) peers))
           (list :peers peers))
  :body (jabber-mam-sync--merge it))
