;;; romacs-bundled-autoloads.el --- The bundled packages' autoloads  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The bundled packages are installed, not preloaded.  Loading this file
;; defines their autoloads, as `package-activate-all' loads installed
;; packages' autoload files before site and user init.
;;
;; Each directory lisp/bundled.json declares (a root, then its immediate
;; subdirectories but the excluded ones, in `load-path' order) is scanned
;; with GNU's own scraper, `loaddefs-generate--parse-file', as package.el
;; scans an installed package: the package's own directory holds the
;; output, so each autoload names its library by bare load name.  The
;; forms are evaluated in the order the loaddefs file `loaddefs-generate'
;; would write holds them; nothing is written.
;;
;; The --loadup launch loads this file into its world
;; (crates/romacs-vm-gui/src/startup.rs), so a world build scans once, and
;; the files and directory listings the scan reads key the world: a bundled
;; file changed, added or removed builds it again.
;;
;; The scan loads no package.  GNU's scraper loads a file whose cookie
;; starts with a symbol it does not know, to learn a macro the file
;; defines; here the scanned files count as files that failed to load, so a
;; world never preloads a package, and a cookie that only its own
;; package's macro can expand signals when its form is evaluated.  The
;; cookies' definers from cl-lib (`cl-defun' and its kin) are known before
;; the scan, so they expand the same in Romacs's world and in `emacs -Q'
;; (the harnesses give pinned GNU this library too).

;;; Code:

(require 'cl-lib)
(require 'loaddefs-gen)

(defun romacs-bundled-autoloads-directories (root)
  "The bundled package directories under ROOT, the runtime root.
Each root lisp/bundled.json declares, then its immediate subdirectories
but the excluded ones, sorted: the order they follow GNU's `load-path' in."
  (let ((layout (with-temp-buffer
                  (insert-file-contents (expand-file-name "lisp/bundled.json" root))
                  (json-parse-buffer :object-type 'alist :array-type 'list)))
        (found nil))
    (dolist (entry (alist-get 'roots layout))
      (let ((base (expand-file-name (alist-get 'path entry) root)))
        (when (file-directory-p base)
          (push base found)
          (dolist (name (directory-files base nil directory-files-no-dot-files-regexp))
            (let ((sub (expand-file-name name base)))
              (when (and (file-directory-p sub)
                         (not (member name (alist-get 'exclude entry))))
                (push sub found)))))))
    (nreverse found)))

(defun romacs-bundled-autoloads--files (dir)
  "DIR's Lisp files `loaddefs-generate' scans, by library name."
  (sort (seq-remove (lambda (file)
                      (equal (file-name-base file) "romacs-bundled-autoloads"))
                    (directory-files dir t "\\`[^=.].*\\.el\\'"))
        (lambda (a b)
          (string< (file-name-sans-extension (file-name-nondirectory a))
                   (file-name-sans-extension (file-name-nondirectory b))))))

(defun romacs-bundled-autoloads--forms (dir files)
  "The autoload forms of FILES, DIR's, in their loaddefs file's order.
A form written to another file (`generated-autoload-file') is left out,
as package.el leaves it."
  (let ((output (expand-file-name "=loaddefs.el" dir))
        (forms nil))
    (dolist (file files)
      (dolist (def (reverse (loaddefs-generate--parse-file file output)))
        (when (equal (expand-file-name (car def)) output)
          (let ((form (nth 2 def)))
            (if (not (stringp form))
                (push form forms)
              ;; Text the scraper copies as it found it: its forms, up to
              ;; the end of the text.
              (let ((at 0))
                (condition-case nil
                    (while t
                      (let ((read (read-from-string form at)))
                        (push (car read) forms)
                        (setq at (cdr read))))
                  (end-of-file nil))))))))
    (nreverse forms)))

(defun romacs-bundled-autoloads-forms (root)
  "The autoload forms of the bundled packages under ROOT, in order.
The scan keeps its state to itself, as a world build, which refuses a
write to what startup read, needs: in a buffer of its own, it binds the
coding its reads set, the hooks its buffers' Lisp mode would run or add
to (it needs Lisp syntax, not the mode's hooks), and the message log its
notices would go to; the notices are not shown either."
  (with-temp-buffer
    ;; Bound here, where none is local, so each binding is the default
    ;; every scanned buffer sees.
    (let* ((last-coding-system-used nil)
           (buffer-file-coding-system buffer-file-coding-system)
           (emacs-lisp-mode-hook nil)
           (lisp-data-mode-hook nil)
           (prog-mode-hook nil)
           (change-major-mode-after-body-hook nil)
           (after-change-major-mode-hook nil)
           (before-change-functions nil)
           (message-log-max nil)
           (inhibit-message t)
           (dirs (romacs-bundled-autoloads-directories root))
           (files (mapcar #'romacs-bundled-autoloads--files dirs))
           (loaddefs--load-error-files (apply #'append files)))
      (apply #'append (seq-mapn #'romacs-bundled-autoloads--forms dirs files)))))

(dolist (form (romacs-bundled-autoloads-forms
               (expand-file-name "../.." (file-name-directory (or load-file-name buffer-file-name)))))
  (eval form t))

(provide 'romacs-bundled-autoloads)

;;; romacs-bundled-autoloads.el ends here
