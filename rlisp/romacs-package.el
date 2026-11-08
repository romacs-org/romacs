;;; romacs-package.el --- GNU's package.el, off the main VM  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; package.el and package-vc.el install packages.  Romacs changes what they
;; do in three small ways, and nothing else:
;;
;; - In a session, a command the user runs that downloads goes to a worker VM,
;;   one at a time, so typing goes on; the session takes in what is on disk
;;   when it ends, and says so.  Lisp callers, `use-package' among them, and
;;   batch and startup run it where they call it, as GNU's do, since they
;;   expect the package afterwards.
;; - Nothing is byte compiled (`package--compile' goes away): a package loads
;;   from its source.
;; - The list of selected packages lives beside the packages, since nothing
;;   writes to the init.

;;; Code:

(require 'package)
(require 'cl-lib)

(defvar package-vc-selected-packages)
(declare-function isolate-spawn "isolate")
(declare-function isolate-on-done "isolate")
(declare-function isolate-await "isolate")
(declare-function isolate-main-p "isolate")
(declare-function romacs-layer-notify "romacs-layer")

(defconst romacs-package--settings
  '(package-archives package-archive-priorities package-pinned-packages package-user-dir
    package-gnupghome-dir package-directory-list package-load-list package-check-signature
    package-selected-packages package-vc-selected-packages package-install-upgrade-built-in
    package-retention-policy url-proxy-services)
  "The options a worker installs under, as the session has them.")

(defun romacs-package--selected-file ()
  (expand-file-name "selected-packages.eld" package-user-dir))

(defun romacs-package--save-selected (&optional value)
  "`package--save-selected-packages', beside the packages.
VALUE nil is valid after startup: the last package was deleted."
  (when (or value after-init-time) (setq package-selected-packages value))
  ;; A worker reports what it changed (`romacs-package--job'); the session saves.
  (when (isolate-main-p)
    (make-directory package-user-dir t)
    (with-temp-file (romacs-package--selected-file)
      (prin1 package-selected-packages (current-buffer)))))

(defun romacs-package--read-selected ()
  "The selected packages saved beside the packages, unless the user set them."
  (let ((file (romacs-package--selected-file)))
    (when (and (null package-selected-packages) (file-readable-p file))
      (setq package-selected-packages
            (with-temp-buffer (insert-file-contents file) (read (current-buffer)))))))

(defun romacs-package--settled (&optional changes)
  "The session takes in what a worker left on disk: the descriptors and the
archives' contents, and CHANGES to the selected packages, as (ADDED . REMOVED).
A package with a new directory activates, reloading what the old one had loaded
while that one is still on `load-path', as GNU's reload needs; then a directory
gone leaves it."
  (let ((before (cl-loop for (name desc) in package-alist
                         collect (cons name (package-desc-dir desc)))))
    (when changes
      (romacs-package--save-selected
       (cl-set-difference (cl-union (car changes) package-selected-packages)
                          (cdr changes))))
    (setq package-alist nil)
    (package-read-all-archive-contents)
    (package-load-all-descriptors)
    (pcase-dolist (`(,name ,desc) package-alist)
      (let ((old (alist-get name before)))
        (unless (equal old (package-desc-dir desc))
          (package-activate-1 desc (and old t) t))))
    (pcase-dolist (`(,name . ,dir) before)
      (unless (equal dir (let ((new (cadr (assq name package-alist))))
                           (and new (package-desc-dir new))))
        (setq load-path (delete (directory-file-name dir) (delete dir load-path)))))
    (add-hook 'post-command-hook #'package-menu--post-refresh)))

(defun romacs-package--crossing (argument)
  "ARGUMENT as it crosses to a worker: a descriptor by its name and version,
in a list too."
  (cond ((package-desc-p argument)
         (list :package (package-desc-name argument) (package-desc-version argument)))
        ((consp argument) (mapcar #'romacs-package--crossing argument))
        (t argument)))

(defun romacs-package--crossed (argument)
  "ARGUMENT back from `romacs-package--crossing', on the worker's descriptors."
  (cond ((eq (car-safe argument) :package)
         (pcase-let ((`(,_ ,name ,version) argument))
           (cl-find version (append (cdr (assq name package-alist))
                                    (cdr (assq name package-archive-contents)))
                    :key #'package-desc-version :test #'equal)))
        ((consp argument) (mapcar #'romacs-package--crossed argument))
        (t argument)))

(defun romacs-package--job (command args)
  "In a worker: run GNU's COMMAND on ARGS; what it changed in the selected
packages, as (ADDED . REMOVED): the session's list may have moved meanwhile."
  (package-initialize)
  (let ((seed package-selected-packages))
    (apply command (mapcar #'romacs-package--crossed args))
    (cons (cl-set-difference package-selected-packages seed)
          (cl-set-difference seed package-selected-packages))))

(defconst romacs-package--done
  '((package-install . "installed") (package-vc-install . "installed")
    (package-upgrade . "upgraded"))
  "What a command says of the package it worked on.")

(defun romacs-package--subject (argument)
  "The name to show for ARGUMENT, a package or its git URL."
  (capitalize (cond ((stringp argument) (file-name-base argument))
                    ((package-desc-p argument) (symbol-name (package-desc-name argument)))
                    (t (format "%s" argument)))))

(defun romacs-package--done-message (command args)
  "What the session says when COMMAND on ARGS has ended well."
  (let ((verb (alist-get command romacs-package--done)))
    (if verb
        (format "%s %s \N{WHITE HEAVY CHECK MARK}" (romacs-package--subject (car args)) verb)
      "Packages updated \N{WHITE HEAVY CHECK MARK}")))

(defun romacs-package--notify (text &optional kind)
  "Say TEXT, which the user asked for a while ago: in *Messages*, and as a
toast that outlasts their typing."
  (let ((inhibit-message (fboundp 'romacs-layer-notify))) (message "%s" text))
  (when (fboundp 'romacs-layer-notify) (romacs-layer-notify text kind)))

(defun romacs-package--spawn (command args &optional prefix)
  "Run COMMAND on ARGS in a worker VM, with the prefix argument PREFIX: its future."
  (isolate-spawn 'romacs-package--job command (mapcar #'romacs-package--crossing args)
                 :name (symbol-name command)
                 :require '(romacs-package package-vc)
                 :allow-source-boot t
                 :bindings (cons (cons 'current-prefix-arg prefix)
                                 (cl-loop for s in romacs-package--settings
                                          when (boundp s) collect (cons s (symbol-value s))))))

(defvar romacs-package--queue nil
  "The (COMMAND PREFIX . ARGS) waiting for a worker; one runs at a time, as they share
the package directory and the selected list.")

(defvar romacs-package--running nil
  "The future of the job a worker is running, or nil.")

(defun romacs-package--next ()
  "Start the next waiting job, unless one is running."
  (when (and romacs-package--queue (not romacs-package--running))
    (pcase-let ((`(,command ,prefix . ,args) (pop romacs-package--queue)))
      (setq romacs-package--running (romacs-package--spawn command args prefix))
      (isolate-on-done romacs-package--running
                       (lambda (done) (romacs-package--finish done command args))))))

(defun romacs-package--finish (done command args)
  "DONE, the future of COMMAND on ARGS, ended: take in the disk, whatever it
left, and say how it went."
  (setq romacs-package--running nil)
  (condition-case err
      (let ((changes (isolate-await done)))
        (romacs-package--settled changes)
        (romacs-package--notify (romacs-package--done-message command args)))
    (error
     (ignore-errors (romacs-package--settled))
     (romacs-package--notify
      (format "%s failed: %s" (if args (romacs-package--subject (car args)) command)
              (error-message-string err))
      'error)))
  (romacs-package--next))

(defun romacs-package--session-p ()
  "Whether this is a running session's main VM, where a worker can take a job."
  (and after-init-time (not noninteractive) (fboundp 'isolate-main-p) (isolate-main-p)))

(defun romacs-package--dispatch (command original &rest args)
  "The user running COMMAND in a session gets a worker, else GNU's call."
  (if (not (and (romacs-package--session-p)
                (or (eq this-command command)
                    (eq command 'package-menu--perform-transaction))))
      (apply original args)
    ;; Its question must be asked here: a worker has no user.
    (when (eq command 'package-upgrade-all)
      (unless (or (null (car args)) (yes-or-no-p "Refresh the lists and upgrade all packages? "))
        (user-error "Nothing done"))
      (setq args nil))
    (setq romacs-package--queue (append romacs-package--queue (list (cons command (cons current-prefix-arg args)))))
    (romacs-package--next)
    nil))

(defun romacs-package--archives-initialize (original)
  "`package--archives-initialize': a session with no lists yet fetches them in a
worker, whose errors are the caller's, and goes on when it has them."
  (when (and (romacs-package--session-p) (null package-archive-contents))
    (unless package--initialized (package-initialize))
    (unless package-archive-contents
      (message "Fetching the package lists...")
      (romacs-package--settled (isolate-await (romacs-package--spawn 'package-refresh-contents nil)))))
  (funcall original))

(dolist (command '(package-install package-upgrade package-upgrade-all package-vc-install
                   package-menu--perform-transaction))
  (advice-add command :around (apply-partially #'romacs-package--dispatch command)))

(advice-add 'package--archives-initialize :around #'romacs-package--archives-initialize)
(advice-add 'package--compile :override #'ignore)
(advice-add 'package--save-selected-packages :override #'romacs-package--save-selected)
(romacs-package--read-selected)

(provide 'romacs-package)
;;; romacs-package.el ends here
