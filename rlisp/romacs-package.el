;;; romacs-package.el --- GNU's package.el, off the main VM  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; package.el and package-vc.el install packages.  Romacs changes what they
;; do in three small ways, and nothing else:
;;
;; - In a session, a command that downloads runs in a worker VM, so typing
;;   goes on; the session activates what it installed when it ends.  In
;;   batch and during startup the command runs where it is called, as GNU's
;;   does, since its caller expects the package afterwards.
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
    package-selected-packages package-vc-selected-packages url-proxy-services)
  "The options a worker installs under, as the session has them.")

(defun romacs-package--selected-file ()
  (expand-file-name "selected-packages.eld" package-user-dir))

(defun romacs-package--save-selected (&optional value)
  "`package--save-selected-packages', beside the packages."
  (when value (setq package-selected-packages value))
  (make-directory package-user-dir t)
  (with-temp-file (romacs-package--selected-file)
    (prin1 package-selected-packages (current-buffer))))

(defun romacs-package--read-selected ()
  "The selected packages saved beside the packages, unless the user set them."
  (let ((file (romacs-package--selected-file)))
    (when (and (null package-selected-packages) (file-readable-p file))
      (setq package-selected-packages
            (with-temp-buffer (insert-file-contents file) (read (current-buffer)))))))

(defun romacs-package--settled (&optional selected)
  "The session takes in what a worker left on disk: SELECTED, the descriptors
and the archives' contents; it activates the new packages."
  (when selected (setq package-selected-packages selected))
  (package-read-all-archive-contents)
  (package-load-all-descriptors)
  (package-activate-all))

(defun romacs-package--job (command args)
  "In a worker: run GNU's COMMAND on ARGS; the selected packages."
  (package-initialize)
  (apply command args)
  package-selected-packages)

(defun romacs-package--argument (arg)
  "ARG as it crosses to a worker: a package by its name."
  (if (package-desc-p arg) (package-desc-name arg) arg))

(defconst romacs-package--done
  '((package-install . "installed") (package-vc-install . "installed")
    (package-upgrade . "upgraded") (package-vc-upgrade . "upgraded"))
  "What a command says of the package it worked on.")

(defun romacs-package--subject (argument)
  "The name to show for ARGUMENT, a package or its git URL."
  (capitalize (if (stringp argument) (file-name-base argument) (format "%s" argument))))

(defun romacs-package--done-message (command args)
  "What the session says when COMMAND on ARGS has ended well."
  (let ((verb (alist-get command romacs-package--done)))
    (cond (verb (format "%s %s \N{WHITE HEAVY CHECK MARK}" (romacs-package--subject (car args)) verb))
          ((eq command 'package-refresh-contents) "Package lists updated \N{WHITE HEAVY CHECK MARK}")
          (t "Packages updated \N{WHITE HEAVY CHECK MARK}"))))

(defun romacs-package--notify (text &optional kind)
  "Say TEXT, which the user asked for a while ago: in *Messages*, and as a
toast that outlasts their typing."
  (let ((inhibit-message (fboundp 'romacs-layer-notify))) (message "%s" text))
  (when (fboundp 'romacs-layer-notify) (romacs-layer-notify text kind)))

(defvar romacs-package--refresh nil
  "The future of the package lists being fetched, or nil.")

(defun romacs-package--spawn (command args)
  "Run COMMAND on ARGS in a worker VM: its future.
The session says so when it ends, and takes the result in."
  (let* ((names (mapcar #'romacs-package--argument args))
         (future (isolate-spawn 'romacs-package--job command names
                                :name (symbol-name command)
                                :require '(romacs-package package-vc)
                                :allow-source-boot t
                                :bindings (cl-loop for s in romacs-package--settings
                                                   when (boundp s) collect (cons s (symbol-value s))))))
    (when (eq command 'package-refresh-contents) (setq romacs-package--refresh future))
    (isolate-on-done
     future
     (lambda (done)
       (condition-case err
           (progn (romacs-package--settled (isolate-await done))
                  (romacs-package--notify (romacs-package--done-message command names)))
         (error (romacs-package--notify
                 (format "%s failed: %s" (if names (romacs-package--subject (car names)) command)
                         (error-message-string err))
                 'error)))))
    future))

(defun romacs-package--await-lists (&rest _)
  "After `package--archives-initialize': a command about to read a package's
name waits for the lists it started fetching.  The display stays live."
  (while (and romacs-package--refresh (not (isolate-future-done-p romacs-package--refresh)))
    (sit-for 0.05))
  (setq romacs-package--refresh nil))

(defun romacs-package--dispatch (command original &rest args)
  "Run COMMAND in a worker from a running session, else as GNU does."
  (if (and after-init-time (not noninteractive)
           (fboundp 'isolate-main-p) (isolate-main-p))
      (romacs-package--spawn command args)
    (apply original args)))

(dolist (command '(package-install package-upgrade package-upgrade-all package-refresh-contents
                   package-install-selected-packages package-vc-install package-vc-upgrade
                   package-vc-upgrade-all))
  (advice-add command :around (apply-partially #'romacs-package--dispatch command)))

(advice-add 'package--archives-initialize :after #'romacs-package--await-lists)
(advice-add 'package--compile :override #'ignore)
(advice-add 'package--save-selected-packages :override #'romacs-package--save-selected)
(romacs-package--read-selected)

(provide 'romacs-package)
;;; romacs-package.el ends here
