;;; session-image-locks.el --- scaling probe  -*- lexical-binding: t -*-

;; The restore's lock pass over N modified file buffers.
(scaling-probe session-image-locks :n 100
  :setup (let ((dir (make-temp-file "scaling-locks-" t)))
           (require 'warm-restart)
           (mapcar (lambda (name)
                     (let ((file (expand-file-name name dir)))
                       (with-temp-file file (insert "text\n"))
                       (with-current-buffer (let ((create-lockfiles nil)) (find-file-noselect file))
                         (let ((create-lockfiles nil)) (insert "edit"))
                         (current-buffer))))
                   (scaling--names "scaling-lock" n)))
  :body (warm-restart--lock it))
