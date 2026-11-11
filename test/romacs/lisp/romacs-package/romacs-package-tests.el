;;; romacs-package-tests.el --- GNU's installer, off the main VM  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Against GNU's own package-resources archive, so no network.

;;; Code:

(require 'ert)
(require 'package)

(defconst romacs-package-tests--archive
  (expand-file-name "../../../test/gnu/test/lisp/emacs-lisp/package-resources/"
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

(ert-deftest romacs-package-settling-twice-leaves-one-descriptor ()
  (romacs-package-tests--with-archive
    (package-install 'simple-single)
    (romacs-package--settled)
    (romacs-package--settled)
    (should (= 1 (length (cdr (assq 'simple-single package-alist)))))))

(ert-deftest romacs-package-deleting-the-last-empties-the-selection ()
  (romacs-package-tests--with-archive
    (let ((after-init-time (current-time)))
      (package-install 'simple-single)
      (package-delete (cadr (assq 'simple-single package-alist)))
      (should-not package-selected-packages)
      (should-not (with-temp-buffer
                    (insert-file-contents (romacs-package--selected-file))
                    (read (current-buffer)))))))

(ert-deftest romacs-package-a-descriptor-crosses-by-name-and-version ()
  (romacs-package-tests--with-archive
    (let* ((desc (cadr (assq 'simple-single package-archive-contents)))
           (crossing (romacs-package--crossing (list desc 'other))))
      (should (eq (romacs-package--crossed (car crossing)) desc))
      (should (eq (cadr crossing) 'other)))))

(ert-deftest romacs-package-lisp-callers-get-gnus-call ()
  "A Lisp caller, and anything out of a session, expects the package afterwards."
  (romacs-package-tests--with-archive
    (let ((this-command 'other))
      (package-install 'simple-single)
      (should (package-installed-p 'simple-single)))))

(defun romacs-package-tests--write (name version result)
  "An installed package NAME at VERSION whose function NAME returns RESULT."
  (let ((dir (expand-file-name (format "%s-%s" name version) package-user-dir)))
    (make-directory dir t)
    (with-temp-file (expand-file-name (format "%s-pkg.el" name) dir)
      (prin1 `(define-package ,(symbol-name name) ,version "doc" nil) (current-buffer)))
    (with-temp-file (expand-file-name (format "%s-autoloads.el" name) dir)
      (prin1 '(add-to-list 'load-path (directory-file-name (file-name-directory load-file-name)))
             (current-buffer)))
    (with-temp-file (expand-file-name (format "%s.el" name) dir)
      (prin1 `(progn (defun ,name () ',result) (provide ',name)) (current-buffer)))
    dir))

(ert-deftest romacs-package-an-upgrade-reloads-what-the-old-one-loaded ()
  "The session runs the new code, with the old directory off `load-path'."
  (romacs-package-tests--with-archive
    (let ((name (intern (format "rp-up-%d" (random 1000000)))))
      (let ((old (romacs-package-tests--write name "1.0" 'one)))
        (romacs-package--settled)
        (require name)
        (should (eq (funcall name) 'one))
        (romacs-package-tests--write name "2.0" 'two)
        (delete-directory old t)
        (romacs-package--settled)
        (should (eq (funcall name) 'two))
        (should-not (member old load-path))
        (should-not (member (directory-file-name old) load-path))))))

(ert-deftest romacs-package-a-job-changes-the-selection-it-changed ()
  "A selection made while a job ran survives its result."
  (romacs-package-tests--with-archive
    (let ((after-init-time (current-time)))
      (setq package-selected-packages '(made-meanwhile dropped))
      (romacs-package--settled '((installed) . (dropped)))
      (should (memq 'installed package-selected-packages))
      (should (memq 'made-meanwhile package-selected-packages))
      (should-not (memq 'dropped package-selected-packages)))))

(provide 'romacs-package-tests)
;;; romacs-package-tests.el ends here
