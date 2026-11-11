;;; process-environment-block.el --- scaling probe  -*- lexical-binding: t -*-

;; A child's environment (lisp/romacs/vm/subprocess.el romacs--environment-block):
;; N distinct names in `process-environment', each first definition kept.
(scaling-probe process-environment-block :n 2000
  :setup (cl-loop for i below n collect (format "SCALING_ENV_%d=value" i))
  :body (let ((process-environment it))
          (romacs--environment-block "/")))
