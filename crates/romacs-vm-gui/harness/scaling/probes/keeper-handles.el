;;; keeper-handles.el --- scaling probe  -*- lexical-binding: t -*-

;; N handles in a keeper of the probe's own: listing them, then a request
;; and a close for each.
(scaling-probe keeper-handles :n 100
  :setup (progn
           (require 'romacs-keeper)
           (let ((romacs-keeper-use-systemd nil)
                 (dir (make-temp-file "rk" t)))
             (romacs-keeper--attach-socket (expand-file-name "keeper.sock" dir))
             (add-hook 'kill-emacs-hook (lambda () (ignore-errors (romacs-keeper-shutdown)) (delete-directory dir t)))
             (mapcar (lambda (_) (plist-get (romacs-keeper-spawn "probe" 1 '("/bin/sleep" "600")) :handle))
                     (number-sequence 1 n))))
  :body (progn
          (unless (= (length (romacs-keeper-handles)) n) (error "Handles missing"))
          (dolist (handle it)
            (romacs-keeper-set-metadata handle "probed")
            (romacs-keeper-close handle 9))))
