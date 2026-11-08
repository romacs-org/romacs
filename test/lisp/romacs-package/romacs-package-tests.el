;;; romacs-package-tests.el --- GNU's installer, off the main VM  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Against GNU's own package-resources archive, so no network.

;;; Code:

(require 'ert)
(require 'package)

(defconst romacs-package-tests--archive
  (expand-file-name "../emacs-lisp/package-resources/"
                    (file-name-directory (or load-file-name buffer-file-name))))

(defmacro romacs-package-tests--with-archive (&rest body)
  "Run BODY with a private package directory and GNU's test archive."
  (declare (indent 0))
  `(let* ((package-user-dir (make-temp-file "romacs-package-" t))
          (package-gnupghome-dir (expand-file-name "gnupg" package-user-dir))
          (package-archives `(("test" . ,romacs-package-tests--archive)))
          (package-check-signature nil)
          (package-selected-packages nil)
          (package-alist nil)
          (package-activated-list nil)
          (package-archive-contents nil)
          (load-path load-path))
     (unwind-protect
         (progn (package-refresh-contents) (package-read-all-archive-contents) ,@body)
       (delete-directory package-user-dir t))))

(defun romacs-package-tests--elc-files ()
  (directory-files-recursively package-user-dir "\\.elc\\'"))

(ert-deftest romacs-package-installs-with-gnus-code-and-compiles-nothing ()
  (romacs-package-tests--with-archive
    (package-install 'simple-single)
    (should (package-installed-p 'simple-single))
    (should (require 'simple-single nil t))
    (should-not (romacs-package-tests--elc-files))))

(ert-deftest romacs-package-saves-selected-beside-the-packages ()
  (romacs-package-tests--with-archive
    (package-install 'simple-single)
    (should (memq 'simple-single package-selected-packages))
    (should (equal (with-temp-buffer
                     (insert-file-contents (romacs-package--selected-file))
                     (read (current-buffer)))
                   package-selected-packages))))

(ert-deftest romacs-package-installs-in-a-worker ()
  "A worker installs; the session takes the result in when it ends.
A worker needs a world to adopt: harness/packages_live.py runs the session."
  (skip-unless (eq isolate-boot 'world))
  (romacs-package-tests--with-archive
    (should-not (package-installed-p 'simple-single))
    (let ((future (romacs-package--spawn 'package-install '(simple-single))))
      (should (isolate-await future 120))
      (should-not (package-installed-p 'simple-single))
      (romacs-package--settled (isolate-await future 120))
      (should (package-installed-p 'simple-single))
      (should (memq 'simple-single package-selected-packages))
      (should (require 'simple-single nil t))
      (should-not (romacs-package-tests--elc-files)))))

(ert-deftest romacs-package-worker-failure-reaches-the-session ()
  (skip-unless (eq isolate-boot 'world))
  (romacs-package-tests--with-archive
    (let ((future (romacs-package--spawn 'package-install '(no-such-package))))
      (should-error (isolate-await future 120))
      (should-not (package-installed-p 'no-such-package)))))

(provide 'romacs-package-tests)
;;; romacs-package-tests.el ends here
