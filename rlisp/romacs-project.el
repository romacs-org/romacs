;;; romacs-project.el --- Project enumeration and search in a worker isolate  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; `project-find-file' and `project-find-regexp' as GNU defines them, with
;; the slow part, `project-files' and `xref-matches-in-files', run in a
;; worker isolate of this session's own world (rlisp/vm/isolate.el).  The
;; command returns to the command loop at once; when the worker's frozen
;; result arrives at a wait point the command continues exactly where GNU's
;; would be: the completion prompt, or the xref buffer.  The candidate list
;; and the matches are the same functions over the same project, so they
;; are what GNU shows; only the moment the prompt or buffer appears differs
;; (DIVERGENCES.md VM2-PROJECT-ISOLATE).  Keys typed meanwhile run their
;; commands instead of waiting.
;;
;; Without a world to spawn from (a source boot, batch) or for a remote
;; project, GNU's definitions run unchanged.

;;; Code:

(defvar romacs-project-isolate t
  "Whether project enumeration and search run in a worker isolate.
t: when this session restored from a world; `source': also when it booted
from source (the worker then boots from source too, seconds); nil: never.")

(defun romacs-project--isolate-available-p (project)
  "Whether PROJECT's enumeration and search may run in a worker."
  (and romacs-project-isolate
       (fboundp 'isolate-spawn)
       (or (and (eq isolate-boot 'world) isolate-world)
           (and (eq isolate-boot 'source) (eq romacs-project-isolate 'source)))
       (not (file-remote-p (project-root project)))))

(defun romacs-project--features (project)
  "What a worker needs loaded to enumerate and search PROJECT: project,
xref and grep, and the library of PROJECT's VC backend, which the session
loaded while finding the project and the worker has not."
  (append '(project xref grep)
          (and (eq (car-safe project) 'vc) (symbolp (nth 1 project)) (nth 1 project)
               (list (intern (format "vc-%s" (downcase (symbol-name (nth 1 project)))))))))

(defun romacs-project--spawn (project function &rest args)
  "Spawn FUNCTION with ARGS on a worker ready for PROJECT."
  (apply #'isolate-spawn function
         (append args (list :require (romacs-project--features project)
                            :allow-source-boot (eq isolate-boot 'source)))))

;;;; The jobs, run in the worker

;; Lambda forms, not symbols: a worker adopts the session's world or boots
;; as it did, and neither holds this module (DIVERGENCES.md
;; VM2-ISOLATE-GLOBALS), so each job carries its definition.

(defconst romacs-project--files-job
  '(lambda (project dirs)
     (let ((project-files-relative-names nil))
       (project-files project dirs)))
  "The files of PROJECT in DIRS, as `project-find-file-in' lists them.")

(defconst romacs-project--matches-job
  '(lambda (regexp project)
     (let ((default-directory (project-root project))
           (project-files-relative-names t))
       (xref-matches-in-files regexp (project-files project))))
  "The matches of REGEXP in PROJECT, as `project-find-regexp' finds them.")

;;;; The commands

(defun romacs-project-find-file (original &optional include-all)
  "Around `project-find-file': enumerate in a worker, then complete and visit.
ORIGINAL runs as GNU's does with INCLUDE-ALL, or without a worker."
  (let ((project (project-current t)))
    (if (or include-all (not (romacs-project--isolate-available-p project)))
        (funcall original include-all)
      (let* ((suggested (thing-at-point 'filename))
             (dirs (list (project-root project)))
             (buffer (current-buffer))
             (window (selected-window))
             (future (condition-case nil
                         (romacs-project--spawn project romacs-project--files-job project dirs)
                       (isolate-queue-full nil))))
        (if (null future)
            (funcall original include-all)
          (isolate-on-done future
                           (lambda (future)
                             (romacs-project--find-file-continue future project suggested buffer window)))
          (message "Finding files in %s..." (project-root project))
          nil)))))

(defun romacs-project--find-file-continue (future project suggested buffer window)
  "`project-find-file-in' from its file list, FUTURE's value, in the WINDOW
and BUFFER it was invoked from, as the prompt and the visit would be."
  (condition-case err
      (with-selected-window (if (window-live-p window) window (selected-window))
        (with-current-buffer (if (buffer-live-p buffer) buffer (current-buffer))
          (let* ((all-files (thaw (isolate-await future)))
                 (completion-ignore-case read-file-name-completion-ignore-case)
                 (default-directory (project-root project))
                 (file (project--read-file-name
                        project "Find file" all-files nil 'file-name-history suggested)))
            (if (string-empty-p file)
                (user-error "You didn't specify the file")
              (find-file file)))))
    (quit nil)
    (error (message "%s" (error-message-string err)))))

(defun romacs-project-find-regexp (original regexp)
  "Around `project-find-regexp': search in a worker, then show the xrefs.
ORIGINAL runs as GNU's does with a prefix argument, or without a worker."
  (let ((project (project-current t)))
    (if (or current-prefix-arg (not (romacs-project--isolate-available-p project)))
        (funcall original regexp)
      (require 'xref)
      (require 'grep)
      (let* ((root (project-root project))
             (buffer (current-buffer))
             (window (selected-window))
             (future (condition-case nil
                         (romacs-project--spawn project romacs-project--matches-job regexp project)
                       (isolate-queue-full nil))))
        (if (null future)
            (funcall original regexp)
          (isolate-on-done future
                           (lambda (future)
                             (romacs-project--show-matches future regexp project root buffer window)))
          (message "Searching %s for %s..." root regexp)
          nil)))))

(defun romacs-project--show-matches (future regexp project root buffer window)
  "`project-find-regexp' from its matches, FUTURE's value, where it was invoked."
  (condition-case err
      (let ((matches (thaw (isolate-await future))))
        (unless matches
          (user-error "No matches for: %s" regexp))
        (with-selected-window (if (window-live-p window) window (selected-window))
          (with-current-buffer (if (buffer-live-p buffer) buffer (current-buffer))
            (let ((default-directory root)
                  (first t))
              ;; The fetcher gives the worker's matches once; a revert
              ;; searches again, as GNU's does, here.
              (xref-show-xrefs
               (lambda ()
                 (if first
                     (progn (setq first nil) matches)
                   (project--find-regexp-in-files
                    regexp (let ((default-directory root)
                                 (project-files-relative-names t))
                             (project-files project)))))
               nil)))))
    (quit nil)
    (user-error (message "%s" (error-message-string err)))
    (error (message "%s" (error-message-string err)))))

(when (fboundp 'advice-add)
  (with-eval-after-load 'project
    (advice-add 'project-find-file :around #'romacs-project-find-file)
    (advice-add 'project-find-regexp :around #'romacs-project-find-regexp)))

(provide 'romacs-project)
;;; romacs-project.el ends here
