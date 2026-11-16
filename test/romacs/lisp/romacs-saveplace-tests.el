;;; romacs-saveplace-tests.el --- Places read without TRAMP -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'saveplace)

(ert-deftest romacs-saveplace-remote-places-load-without-tramp ()
  ;; A places file naming files on remote hosts reads without loading
  ;; TRAMP: the first buffer whose mode reads the places (a restored
  ;; dired, a visit) waited a second for TRAMP to load, there and then.
  ;; The remote names stay as written; the local ones are expanded.
  (skip-unless (not (featurep 'tramp)))
  (let* ((file (make-temp-file "romacs-saveplace-" nil ".eld"
                               (prin1-to-string '(("/ssh:user@host:/srv/notes.org" . 10)
                                                  ("~/notes.org" . 20)
                                                  ("/ssh:user@host:/srv/" (dired-filename . "/ssh:user@host:/srv/a"))))))
         (save-place-file file)
         (save-place-loaded nil)
         (save-place-alist nil)
         (save-place-abbreviate-file-names nil))
    (unwind-protect
        (progn
          (save-place-load-alist-from-file)
          (should-not (featurep 'tramp))
          (should (equal save-place-alist
                         `(("/ssh:user@host:/srv/notes.org" . 10)
                           (,(expand-file-name "~/notes.org") . 20)
                           ("/ssh:user@host:/srv/" (dired-filename . "/ssh:user@host:/srv/a"))))))
      (delete-file file))))

;;; romacs-saveplace-tests.el ends here
