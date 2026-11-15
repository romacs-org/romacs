;;; romacs-agent-keys-tests.el --- The agents' global prefix, C-x C-a -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-bundled-autoloads)

;; The agents' prefix is bound by an autoload cookie, as a world binds it:
;; its scraped forms evaluated (romacs-bundled-autoloads).
(let ((file (locate-library "romacs-agent-ui.el")))
  (mapc #'eval (romacs-bundled-autoloads--forms (file-name-directory file) (list file))))

(ert-deftest romacs-agent-keys-prefix-is-c-x-c-a ()
  ;; Global, so under C-x: C-c C-<key> is left to major modes.
  (should (eq (key-binding (kbd "C-x C-a a")) 'romacs-agents))
  (should (eq (key-binding (kbd "C-x C-a n")) 'romacs-agent-start))
  (should-not (keymapp (lookup-key global-map (kbd "C-c C-;")))))

(ert-deftest romacs-agent-keys-share-the-prefix-with-edebug ()
  ;; Edebug's control keys under C-x C-a in an Emacs Lisp buffer, the
  ;; agents' letters beside them.
  (require 'edebug)
  (with-temp-buffer
    (emacs-lisp-mode)
    (should (eq (key-binding (kbd "C-x C-a C-s")) 'edebug-step-mode))
    (should (eq (key-binding (kbd "C-x C-a C-n")) 'edebug-next-mode))
    (should (eq (key-binding (kbd "C-x C-a a")) 'romacs-agents))))

(ert-deftest romacs-agent-keys-share-the-prefix-with-gud ()
  ;; Loading gud binds its global map on C-x C-a: the agents' map stays
  ;; under it, gud's keys first.
  (require 'gud)
  (should (eq (key-binding (kbd "C-x C-a C-l")) 'gud-refresh))
  (should (eq (key-binding (kbd "C-x C-a a")) 'romacs-agents))
  ;; A gud command defined once a debugger runs.
  (define-key gud-global-map (kbd "C-b") #'ignore)
  (unwind-protect
      (progn (should (eq (key-binding (kbd "C-x C-a C-b")) 'ignore))
             (should (eq (key-binding (kbd "C-x C-a n")) 'romacs-agent-start)))
    (define-key gud-global-map (kbd "C-b") nil t)))

;;; romacs-agent-keys-tests.el ends here
