;;; mail-lifecycle-test.el --- User-controlled mail buffer lifetime -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(romacs-deftest mail-dirty-draft-can-be-killed ()
  (with-temp-buffer
    (romacs-mail-compose '(:name "fixture" :from "user@example.invalid"))
    (let ((draft (current-buffer)))
      (insert "Unsent text")
      (romacs-should (buffer-modified-p))
      (romacs-should (kill-buffer draft))
      (romacs-should-not (buffer-live-p draft)))))

(romacs-deftest mail-pending-draft-can-be-killed-without-stale-writes ()
  (with-temp-buffer
    (romacs-mail-compose '(:name "fixture" :from "user@example.invalid"))
    (let ((draft (current-buffer)) (owner romacs-mail-owner) (name (buffer-name)))
      (setq romacs-mail-send-state 'sending buffer-read-only t)
      (romacs-should (kill-buffer draft))
      (with-current-buffer (get-buffer-create name)
        (insert "Replacement document")
        (romacs-mail-settle-send draft owner '(0 "" "" nil))
        (romacs-should (equal (buffer-string) "Replacement document"))
        (kill-buffer (current-buffer))))))

(romacs-deftest mail-draft-does-not-block-explicit-editor-quit ()
  (with-temp-buffer
    (romacs-mail-compose '(:name "fixture" :from "user@example.invalid"))
    (let ((draft (current-buffer))
          (previous-quit romacs-gui-quit-requested)
          (romacs-gui-visits nil))
      (setq romacs-mail-send-state 'sending romacs-gui-quit-requested nil)
      (unwind-protect
          (progn (romacs-gui-quit) (romacs-should romacs-gui-quit-requested))
        (setq romacs-gui-quit-requested previous-quit)
        (let ((kill-buffer-query-functions nil)) (kill-buffer draft))))))

(provide 'mail-lifecycle-test)
