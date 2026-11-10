;;; agent-environment.el --- scaling probe  -*- lexical-binding: t -*-

;; A harness's environment, with N harness drivers defined (each a
;; configuration variable and one it sets for its children) and N other
;; variables in the editor's: one membership test per variable, linear.
(scaling-probe agent-environment :n 500
  :setup (progn (scaling--agent)
                (dotimes (i n)
                  (romacs-agent-define-harness (intern (format "scaling-edriver-%d" i)) :lifetime 'persistent
                                               :config-variable (format "SCALING_EDRIVER_%d_HOME" i)
                                               :scrub (list (format "SCALING_EDRIVER_%d_CHILD" i))))
                (append (cl-loop for i below n collect (format "SCALING_KEPT_%d=value" i))
                        (cl-loop for i below n collect (format "SCALING_EDRIVER_%d_CHILD=parent" i))
                        process-environment))
  :body (let ((process-environment it)) (dotimes (_ 5) (romacs-agent--harness-environment))))
