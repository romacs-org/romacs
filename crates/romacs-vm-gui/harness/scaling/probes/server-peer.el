;;; server-peer.el --- scaling probe  -*- lexical-binding: t -*-

;; N requests held on one connection, then the connection lost.
(scaling-probe server-peer :n 200
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-peer" n)))
  :body (progn
          (dolist (buffer it) (romacs-server--wait (list :probe t) 'scaling-one-peer (list buffer)))
          (romacs-server--peer-closed 'scaling-one-peer)))
