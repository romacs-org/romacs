;;; agent-history-items-drivers.el --- scaling probe  -*- lexical-binding: t -*-

;; The transcripts no session continues, among 200 listed and 200
;; sessions, of a harness defined before N others: each row's identity
;; finds its harness by name, never by a walk of the registry.
(scaling-probe agent-history-items-drivers :n 400 :budget 0.1
  :setup (progn (scaling--agent)
                (require 'romacs-agent-history)
                ;; Defined first: the N after it stand between it and the head.
                (romacs-agent-define-harness 'scaling-hlast :lifetime 'persistent :config-variable "SCALING_HLAST_HOME"
                                             :config-default "/scaling/home")
                (dotimes (i n)
                  (romacs-agent-define-harness (intern (format "scaling-hdriver-%d" i)) :lifetime 'persistent
                                               :config-variable (format "SCALING_HDRIVER_%d_HOME" i)))
                (clrhash romacs-agent--sessions)
                (dotimes (i 200)
                  (let ((id (format "%08x-0000-4000-8000-%012x" i i)))
                    (puthash id (romacs-agent--make :id id :harness 'scaling-hlast :title id :harness-session id)
                             romacs-agent--sessions)))
                (setq romacs-agent-history--listing
                      (cl-loop for i below 200
                               collect (list :harness 'scaling-hlast :id (format "%08x-0000-4000-8000-%012x" (* 2 i) i)
                                             :path (format "/scaling/%d.jsonl" i) :mtime i :config-dir nil)))
                nil)
  :body (dotimes (_ 5) (romacs-agent-history-items)))
