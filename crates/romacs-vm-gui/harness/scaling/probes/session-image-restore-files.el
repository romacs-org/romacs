;;; session-image-restore-files.el --- scaling probe  -*- lexical-binding: t -*-

;; An image of N file buffers, every other one with unsaved text,
;; restored into a session without them.
(scaling-probe session-image-restore-files :n 100
  :setup (let ((dir (make-temp-file "scaling-files-" t)) (create-lockfiles nil))
           (scaling--session 0)
           (dolist (name (scaling--names "scaling-file" n))
             (let ((file (expand-file-name name dir)))
               (with-temp-file file (insert name "\n"))
               (with-current-buffer (find-file-noselect file)
                 (when (cl-evenp (length (buffer-list))) (insert "unsaved\n")))))
           (let ((image (expand-file-name "f.image" warm-restart-directory)))
             (warm-restart-write nil image)
             (dolist (buffer (buffer-list))
               (when (buffer-file-name buffer)
                 (with-current-buffer buffer (set-buffer-modified-p nil))
                 (kill-buffer buffer)))
             image))
  :body (warm-restart-restore-image it))
