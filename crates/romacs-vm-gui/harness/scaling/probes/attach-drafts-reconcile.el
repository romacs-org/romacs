;;; attach-drafts-reconcile.el --- scaling probe  -*- lexical-binding: t -*-

;; A draft naming N files beside N orphans: one listing, linear.
(scaling-probe attach-drafts-reconcile :n 1000 :steps 1
  :setup (progn (require 'romacs-attach)
                (let* ((named (scaling--attach-files n))
                       (dir (file-name-directory (car named))))
                  (cons dir (mapcar (lambda (file) (list :path file)) named))))
  :body (let ((dir (car it)))
          (dotimes (i n) (write-region "o" nil (expand-file-name (format "orphan%d" i) dir) nil 'silent))
          (romacs-attach-reconcile dir (cdr it))))
