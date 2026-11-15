;;; romacs-bundled-autoloads.el --- The bundled packages' autoloads  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The bundled packages are installed, not preloaded.  Loading this file
;; defines their autoloads, as `package-activate-all' loads installed
;; packages' autoload files before site and user init.
;;
;; Romacs's runtime files and the bundled packages are scanned, directory
;; by directory in
;; `load-path' order, with GNU's own scraper, `loaddefs-generate--parse-file',
;; as package.el scans an installed package: the file's own directory holds
;; the output, so each autoload names its library by bare load name.
;; Romacs's runtime files are those in its subsystem directories, every
;; `romacs-' file elsewhere on the load-path, and the files lisp/bundled.json
;; `files' names (tools/lib/lisp_layout.py is_scanned holds the same rule);
;; their cookies run here, after loadup.  Every other file's autoloads are in
;; the loaddefs files tools/lisp-loaddefs makes, which loadup loads.  The
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

;; The layout is read here, not from Rust: the installed copy may differ.
(defun romacs-bundled-autoloads--layout (root)
  (with-temp-buffer
    (insert-file-contents (expand-file-name "lisp/bundled.json" root))
    (json-parse-buffer :object-type 'alist :array-type 'list)))

(defun romacs-bundled-autoloads--subdirectories (dir)
  (cl-remove-if-not #'file-directory-p
                    (directory-files dir t directory-files-no-dot-files-regexp)))

(defun romacs-bundled-autoloads--path (root)
  "The load-path directories under ROOT, the runtime root, in order: lisp/,
GNU's directories, Romacs's subsystem directories, then each package."
  (let* ((layout (romacs-bundled-autoloads--layout root))
         (lisp (expand-file-name "lisp" root))
         (own (append (alist-get 'directories layout) (alist-get 'off_path layout)
                      (list (alist-get 'packages layout))))
         (packages (expand-file-name (alist-get 'packages layout) lisp)))
    (append (list lisp)
            (cl-remove-if (lambda (dir) (member (file-name-nondirectory dir) own))
                          (romacs-bundled-autoloads--subdirectories lisp))
            (cl-remove-if-not #'file-directory-p
                              (mapcar (lambda (name) (expand-file-name name lisp))
                                      (alist-get 'directories layout)))
            (and (file-directory-p packages)
                 (romacs-bundled-autoloads--subdirectories packages)))))

(defun romacs-bundled-autoloads--scan (root)
  "((DIR . FILES) ...): the directories under ROOT with files to scan, in
`load-path' order: all of a Romacs directory's or a package's files, and in
the others only `romacs-' files and those lisp/bundled.json `files' names."
  (let* ((layout (romacs-bundled-autoloads--layout root))
         (lisp (expand-file-name "lisp" root))
         (own (mapcar (lambda (name) (expand-file-name name lisp)) (alist-get 'directories layout)))
         (packages (expand-file-name (alist-get 'packages layout) lisp))
         (named (mapcar (lambda (file) (expand-file-name file root)) (alist-get 'files layout))))
    (cl-loop for dir in (romacs-bundled-autoloads--path root)
             for whole = (or (member dir own) (equal (file-name-directory (directory-file-name dir))
                                                     (file-name-as-directory packages)))
             for files = (cl-remove-if-not
                          (lambda (file)
                            (or whole (string-prefix-p "romacs-" (file-name-nondirectory file))
                                (member file named)))
                          (romacs-bundled-autoloads--files dir))
             when files collect (cons dir files))))

(defun romacs-bundled-autoloads-directories (root)
  "The directories under ROOT, the runtime root, whose files the scan reads,
in the order they follow GNU's `load-path' in."
  (mapcar #'car (romacs-bundled-autoloads--scan root)))

(defun romacs-bundled-autoloads--files (dir)
  "DIR's Lisp files `loaddefs-generate' scans, by library name."
  (sort (cl-remove-if (lambda (file)
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
           (scan (romacs-bundled-autoloads--scan root))
           (loaddefs--load-error-files (apply #'append (mapcar #'cdr scan))))
      (apply #'append (mapcar (lambda (entry) (romacs-bundled-autoloads--forms (car entry) (cdr entry)))
                              scan)))))

(dolist (form (romacs-bundled-autoloads-forms
               (expand-file-name "../.." (file-name-directory (or load-file-name buffer-file-name)))))
  (eval form t))

(provide 'romacs-bundled-autoloads)

;;; romacs-bundled-autoloads.el ends here
