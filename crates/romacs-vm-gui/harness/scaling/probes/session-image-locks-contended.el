;;; session-image-locks-contended.el --- scaling probe  -*- lexical-binding: t -*-

;; The same pass when another editor holds every one of those locks.
(scaling-probe session-image-locks-contended :n 100
  :setup (let ((dir (make-temp-file "scaling-contended-" t)))
           (require 'warm-restart)
           (mapcar (lambda (name)
                     (let ((file (expand-file-name name dir)))
                       (with-temp-file file (insert "text\n"))
                       (make-symbolic-link "someone@elsewhere.1:0" (expand-file-name (concat ".#" name) dir))
                       (with-current-buffer (let ((create-lockfiles nil)) (find-file-noselect file))
                         (let ((create-lockfiles nil)) (insert "edit"))
                         (current-buffer))))
                   (scaling--names "scaling-contended" n)))
  :body (warm-restart--lock it))
