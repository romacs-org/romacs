;;; jabber-sync-merge-repeat.el --- scaling probe  -*- lexical-binding: t -*-

;; N finished jobs' merges, one changed peer and its open buffer each.
(scaling-probe jabber-sync-merge-repeat :n 400
  :setup (let ((jc (scaling--jabber)) summaries)
           (setq jabber-mam-sync--merge-queue nil jabber-mam-sync--merge-tail nil)
           (clrhash jabber-mam-sync--merge-queued)
           (dolist (room (scaling--names "room" n))
             (with-current-buffer (get-buffer-create (concat " *scaling-merge-" room "*"))
               (setq major-mode 'jabber-chat-mode)
               (setq-local jabber-buffer-connection jc)
               (setq-local jabber-group room)
               (jabber-buffer-registry-register 'muc (list "romeo@montague.net" room)))
             (push (list :peers (list (list "romeo@montague.net" room "groupchat" 1 1))) summaries))
           summaries)
  :body (dolist (summary it) (jabber-mam-sync--merge summary)))
