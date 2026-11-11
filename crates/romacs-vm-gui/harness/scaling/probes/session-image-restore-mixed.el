;;; session-image-restore-mixed.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe session-image-restore-mixed :n 50
  :setup (progn
           (scaling--session n)
           (add-to-list 'load-path (expand-file-name "hermes" (file-name-directory (locate-library "warm-restart"))))
           (require 'hermes-chat)
           (scaling--offline-hermes
            (lambda ()
              (let ((warm-restart-passive t) (warm-restart-activate-functions nil))
                (dolist (name (scaling--names "scaling-chat" n))
                  (hermes-chat-desktop-restore nil name (list :version 1 :session-id name :title name))))
              (setq hermes-chat--warm-restart-attaches nil)))
           (let ((file (expand-file-name "m.image" warm-restart-directory)))
             (warm-restart-write nil file)
             (dolist (buffer (buffer-list))
               (when (string-match-p "\\`scaling-\\(chat\\|session\\)" (buffer-name buffer))
                 (with-current-buffer buffer (set-buffer-modified-p nil))
                 (let ((kill-buffer-query-functions nil)) (kill-buffer buffer))))
             file))
  :body (scaling--offline-hermes (lambda () (warm-restart-restore-image it))))
