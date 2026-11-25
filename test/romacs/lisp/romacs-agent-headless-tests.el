;;; romacs-agent-headless-tests.el --- The headless agents service's start -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-agent-headless)

(defvar romacs-agent-headless-tests--ran nil "What the test init's hook did.")

(ert-deftest romacs-agent-headless-takes-the-init-s-after-init-hook ()
  ;; A batch start ran `after-init-hook' before the init: what the init
  ;; puts there runs once it has loaded; an init that fails is told to
  ;; clients.
  (let* ((config (make-temp-file "rah" t))
         (process-environment (cons (concat "XDG_CONFIG_HOME=" config) process-environment))
         (romacs-agent-service-init-failure nil)
         (user-emacs-directory user-emacs-directory) (user-init-file user-init-file)
         (inhibit-message t))
    (unwind-protect
        (progn
          (make-directory (expand-file-name "romacs" config))
          (with-temp-file (expand-file-name "romacs/init.el" config)
            (insert "(add-hook 'after-init-hook (lambda () (setq romacs-agent-headless-tests--ran 'after-init)))\n"))
          (romacs-agent-headless--load-init)
          (should (eq romacs-agent-headless-tests--ran 'after-init))
          (should-not romacs-agent-service-init-failure)
          (with-temp-file (expand-file-name "romacs/init.el" config)
            (insert "(error \"broken init\")\n"))
          (romacs-agent-headless--load-init)
          (should (string-search "broken init" romacs-agent-service-init-failure))
          ;; However much fails, hello tells a bounded part of it.
          (romacs-agent-headless--failed "init" (make-string 100000 ?x))
          (should (<= (length romacs-agent-service-init-failure) romacs-agent-headless--failure-most)))
      (delete-directory config t))))

(provide 'romacs-agent-headless-tests)
;;; romacs-agent-headless-tests.el ends here
