;;; server-waiters-index.el --- scaling probe  -*- lexical-binding: t -*-

;; One native client waiting on N files, each finished with server-done,
;; last first: each costs its own waiters, never the files left.  The
;; index alone (bury-buffer, which every server-done runs as GNU's does,
;; left out) and the whole command.
(scaling-probe server-waiters-index :n 200
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-wait" n)))
  :body (cl-letf (((symbol-function 'romacs-mcp-reply) #'ignore)
                  ((symbol-function 'bury-buffer) #'ignore))
          (romacs-server--wait (list :probe t) 'scaling-peer it)
          (dolist (buffer (reverse it)) (with-current-buffer buffer (server-done)))))
