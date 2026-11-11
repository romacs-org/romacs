;;; agent-instances-drivers.el --- scaling probe  -*- lexical-binding: t -*-

;; N drivers, each a T3 driver name, and T3's settings of N instances of
;; them, read anew: each instance's harness by an index built once.
(scaling-probe agent-instances-drivers :n 500
  :setup (progn (scaling--agent)
                (require 'romacs-agent-t3)
                (dotimes (i n)
                  (romacs-agent-define-harness (intern (format "scaling-tdriver-%d" i)) :lifetime 'persistent
                                               :t3-driver (format "tdriver%d" i)
                                               :config-variable (format "SCALING_TDRIVER_%d_HOME" i)
                                               :config-default (format "/tmp/tdriver-%d" i)))
                (let ((file (make-temp-file "scaling-t3d-" nil ".json"))
                      (instances (make-hash-table :test #'equal)))
                  ;; The first defined is the farthest from the registry's head.
                  (dotimes (i n)
                    (puthash (format "t%d" i) (romacs-mcp--object "driver" (format "tdriver%d" (- n 1 i))
                                                                  "displayName" (format "T %d" i))
                             instances))
                  (with-temp-file file (insert (json-serialize (romacs-mcp--object "providerInstances" instances))))
                  (list (list 't3 :settings file))))
  :body (let ((romacs-agent-providers it))
          (setq romacs-agent-t3--read nil)
          (romacs-agent-instances t)))
