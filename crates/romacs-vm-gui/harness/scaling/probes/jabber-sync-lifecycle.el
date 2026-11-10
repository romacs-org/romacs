;;; jabber-sync-lifecycle.el --- scaling probe  -*- lexical-binding: t -*-

;; N accounts, each through a whole job: requested, started, finished
;; (no worker: the isolate functions answer at once), all retained.
(scaling-probe jabber-sync-lifecycle :n 300
  :setup (progn
           (scaling--jabber)
           (clrhash jabber-mam-sync--accounts)
           (setq jabber-mam-sync--live-jobs 0)
           (mapcar (lambda (name)
                     (let ((jc (make-symbol name)))
                       (put jc :state :session-established)
                       (put jc :state-data (list :username name :server "montague.net"
                                                 :resource "r" :password "pw"))
                       jc))
                   (scaling--names "user" n)))
  :body (let (callbacks)
          (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) 'future))
                    ((symbol-function 'isolate-on-done) (lambda (_f callback) (push callback callbacks)))
                    ((symbol-function 'isolate-await) (lambda (&rest _) '(:archives nil :peers nil)))
                    ((symbol-function 'isolate-future-isolate) (lambda (_) 'worker))
                    ((symbol-function 'isolate-live-p) #'ignore)
                    ((symbol-function 'thaw) #'identity)
                    ((symbol-function 'jabber-mam-sync--toast) #'ignore)
                    ((symbol-function 'jabber-db-share-writes) #'ignore))
            (dolist (jc it)
              (jabber-mam-sync-request jc)
              (jabber-mam-sync--start (jabber-connection-bare-jid jc)))
            (dolist (callback callbacks) (funcall callback 'future)))))
