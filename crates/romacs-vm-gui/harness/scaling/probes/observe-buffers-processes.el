;;; observe-buffers-processes.el --- scaling probe  -*- lexical-binding: t -*-

;; observe's buffer summary over N buffers and N processes, each with
;; its buffer: which has a process is a table made once, not a walk of
;; the processes per buffer.
(scaling-probe observe-buffers-processes :n 200
  :setup (progn (scaling--agent)
                (require 'romacs-mcp-session)
                (dotimes (i n)
                  (let ((buffer (get-buffer-create (format "scaling-observe-%d" i))))
                    (make-pipe-process :name (format "scaling-observe-%d" i) :buffer buffer :noquery t)))
                nil)
  :body (dotimes (_ 5) (romacs-mcp-session--buffers (romacs-mcp--object))))
