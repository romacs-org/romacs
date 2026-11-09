;;; server-waiters.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe server-waiters :n 200
  :known "ticket buffers: bury-buffer, which server-done runs per finished buffer, rebuilds the buffer order list"
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-wait" n)))
  :body (cl-letf (((symbol-function 'romacs-mcp-reply) #'ignore))
          (romacs-server--wait (list :probe t) 'scaling-peer it)
          (dolist (buffer (reverse it)) (with-current-buffer buffer (server-done)))))
