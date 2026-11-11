;;; romacs-project.el --- Project enumeration and search in a worker isolate  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; `project-find-file' and `project-find-regexp' as GNU defines them, with
;; the slow part, `project-files' and `xref-matches-in-files', run in a
;; worker isolate of this session's own world (lisp/romacs/vm/isolate.el) when
;; one stands ready.  The command returns to the command loop at once; when
;; the worker's frozen result arrives at a wait point the command continues
;; exactly where GNU's would be: the completion prompt, or the xref buffer.
;; The candidate list and the matches are the same functions over the same
;; project, so they are what GNU shows; only the moment the prompt or buffer
;; appears differs (DIVERGENCES.md VM2-PROJECT-ISOLATE).  Keys typed
;; meanwhile run their commands instead of waiting.
;;
;; A worker is used only when it is already prepared and idle, or is the
;; child of a zygote already preparing (lisp/romacs/vm/isolate.el), which forks
;; it once ready: a worker's boot (the world's adoption and the libraries'
;; loading) costs about as much as the job, so a command that would have to
;; start one runs GNU's definition instead, and warms a worker for the next
;; command once the session idles.  `project-find-regexp' warms its worker while the user
;; types the regexp, so its first run normally finds the worker ready (a
;; regexp takes longer to type than the worker's half-second boot).  A ready
;; worker that will not take the job after all (its queue full, its process
;; gone since it was found) is passed over the same way.
;;
;; Without a world to spawn from (a source boot, batch) or for a remote
;; project, GNU's definitions run unchanged.

;;; Code:

(defvar romacs-project-isolate t
  "Whether project enumeration and search run in a worker isolate.
t: when this session restored from a world; `source': also when it booted
from source (the worker then boots from source too, seconds); nil: never.")

(defcustom romacs-project-inline-threshold 0.03
  "Seconds of file enumeration within which `project-find-file' enumerates
in the session rather than in a worker.  The estimate is the larger of the
project's last enumeration and its Git index's size at the rate last
measured; with neither, a worker is used.  An inline enumeration blocks
the session while it runs, so keep this within the per-key budget."
  :type 'number
  :group 'project)

(defvar romacs-project--enumerations (make-hash-table :test 'equal)
  "Project root to the seconds its last enumeration took.")

(defvar romacs-project--seconds-per-index-byte 1e-7
  "Enumeration seconds per byte of a Git index, as last measured.")

(defvar romacs-project--timing nil
  "The root of the project whose `project-files' call is being timed.")

(defun romacs-project--index-bytes (root)
  "The size of ROOT's Git index, or nil: in its .git directory, or in the
directory its .git file names (a linked worktree)."
  (let* ((dot (expand-file-name ".git" root))
         (dir (cond ((file-directory-p dot) dot)
                    ((file-regular-p dot)
                     (with-temp-buffer
                       (insert-file-contents dot)
                       (and (re-search-forward "^gitdir: \\(.*\\)$" nil t)
                            (expand-file-name (match-string 1) root))))))
         (attributes (and dir (file-attributes (expand-file-name "index" dir)))))
    (and attributes (file-attribute-size attributes))))

(defun romacs-project--enumeration-estimate (root)
  "The seconds enumerating ROOT's files is expected to take, or nil."
  (let* ((last (gethash root romacs-project--enumerations))
         (bytes (romacs-project--index-bytes root))
         (guess (and bytes (* bytes romacs-project--seconds-per-index-byte))))
    (if (and last guess) (max last guess) (or last guess))))

(defun romacs-project--inline-p (project)
  "Whether PROJECT's files are expected to enumerate within
`romacs-project-inline-threshold': a worker's dispatch and delivery would
cost more than they save."
  (let ((estimate (romacs-project--enumeration-estimate (project-root project))))
    (and estimate (<= estimate romacs-project-inline-threshold))))

(defun romacs-project--record (root seconds &optional calibrate)
  "ROOT's files took SECONDS to enumerate; with CALIBRATE, a measure of the
enumeration alone, which sets the rate per index byte."
  (puthash root seconds romacs-project--enumerations)
  (let ((bytes (and calibrate (romacs-project--index-bytes root))))
    (when (and bytes (> bytes 0))
      (setq romacs-project--seconds-per-index-byte (/ seconds bytes)))))

(defun romacs-project--time-files (files project &rest args)
  "Around `project-files': time the enumeration of the project being timed."
  (if (not (and romacs-project--timing (equal (project-root project) romacs-project--timing)))
      (apply files project args)
    (let ((root romacs-project--timing)
          (romacs-project--timing nil)
          (start (float-time)))
      (prog1 (apply files project args)
        (romacs-project--record root (- (float-time) start) t)))))

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

(defun romacs-project--worker (project)
  "The idle worker prepared for PROJECT's commands, or nil."
  (and (romacs-project--isolate-available-p project)
       (isolate-idle-worker (romacs-project--features project))))

(defun romacs-project--warm (project)
  "Start a worker for PROJECT's commands unless the pool has one.
A worker that cannot start (no world, the pool full) is simply not there:
the commands then run GNU's definitions."
  (when (romacs-project--isolate-available-p project)
    (condition-case nil
        (isolate-warm (romacs-project--features project) (eq isolate-boot 'source))
      (error nil))))

(defun romacs-project--warm-when-idle (project)
  "Warm a worker for PROJECT once the session idles."
  (run-with-idle-timer 0.5 nil #'romacs-project--warm project))

(defun romacs-project--warm-at-prompt (&rest _)
  "Before the regexp prompt: warm a worker for the current project, so the
search that follows the prompt finds it ready."
  (let ((project (condition-case nil (project-current nil) (error nil))))
    (when project (romacs-project--warm project))))

(defun romacs-project--spawn (project worker function &rest args)
  "Spawn FUNCTION with ARGS on WORKER, prepared for PROJECT: its future, or
nil when WORKER will not take the job after all (its queue is full, its
process is gone since it was found ready); the command then runs GNU's
definition."
  (condition-case nil
      (apply #'isolate-spawn function
             (append args (list :isolate worker :require (romacs-project--features project))))
    (isolate-error nil)))

(defun romacs-project--note (format &rest args)
  "Show FORMAT with ARGS in the echo area while a worker runs the job, and
return it.  GNU's synchronous command shows nothing, so it is not logged."
  (let ((message-log-max nil))
    (apply #'message format args)))

(defun romacs-project--resume (note window buffer function)
  "Continue a command by calling FUNCTION in its WINDOW and BUFFER.
Clear the progress NOTE if the echo area still shows it.  WINDOW is
selected and BUFFER made current as the command had them.  The window
FUNCTION selects stays selected and its buffer is current after, as the
command loop leaves them once GNU's command returns (command_loop_1):
`xref-show-xrefs' selects its buffer's window, which then heads the
buffer list, and the keys read next are looked up in that buffer."
  (when (equal (current-message) note)
    (message nil))
  (when (window-live-p window)
    (select-window window 'norecord))
  (when (buffer-live-p buffer)
    (set-buffer buffer))
  (prog1 (funcall function)
    (set-buffer (window-buffer (selected-window)))))

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
  "Around `project-find-file': enumerate in a ready worker, then complete
and visit.  ORIGINAL runs as GNU's does with INCLUDE-ALL, or without a
ready worker, which is then warmed for the next time."
  (let* ((project (project-current t))
         ;; A small project enumerates faster in the session than a worker
         ;; could take the job and give the list back.
         (inline (and (not include-all) (romacs-project--inline-p project)))
         (worker (and (not include-all) (not inline) (romacs-project--worker project)))
         (suggested (and worker (thing-at-point 'filename)))
         (dirs (list (project-root project)))
         (buffer (current-buffer))
         (window (selected-window))
         (start (float-time))
         (future (and worker (romacs-project--spawn project worker romacs-project--files-job project dirs))))
    (if (null future)
        (prog1 (let ((romacs-project--timing (and (not include-all) (project-root project))))
                 (funcall original include-all))
          (unless (or include-all inline) (romacs-project--warm-when-idle project)))
      (let ((note (romacs-project--note "Finding files in %s..." (project-root project))))
        (isolate-on-done future
                         (lambda (future)
                           ;; Dispatch and delivery included: never less than inline.
                           ;; A failed job measured nothing.
                           (when (eq (aref future 3) 'done)
                             (romacs-project--record (project-root project) (- (float-time) start)))
                           (romacs-project--find-file-continue future project suggested note buffer window))))
      nil)))

(defun romacs-project--find-file-continue (future project suggested note buffer window)
  "`project-find-file-in' from its file list, FUTURE's value, in the WINDOW
and BUFFER it was invoked from, as the prompt and the visit would be."
  (condition-case err
      (romacs-project--resume
       note window buffer
       (lambda ()
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
  "Around `project-find-regexp': search in a ready worker, then show the
xrefs.  ORIGINAL runs as GNU's does with a prefix argument, or without a
ready worker, which is then warmed for the next time."
  (let* ((project (project-current t))
         (worker (and (not current-prefix-arg) (romacs-project--worker project)))
         (root (project-root project))
         (buffer (current-buffer))
         (window (selected-window))
         (future (and worker (romacs-project--spawn project worker romacs-project--matches-job regexp project))))
    (if (null future)
        (prog1 (funcall original regexp)
          (unless current-prefix-arg (romacs-project--warm-when-idle project)))
      (require 'xref)
      (require 'grep)
      (let ((note (romacs-project--note "Searching %s for %s..." root regexp)))
        (isolate-on-done future
                         (lambda (future)
                           (romacs-project--show-matches future regexp project root note buffer window))))
      nil)))

(defun romacs-project--show-matches (future regexp project root note buffer window)
  "`project-find-regexp' from its matches, FUTURE's value, where it was invoked."
  (condition-case err
      (let ((matches (thaw (isolate-await future))))
        (unless matches
          (user-error "No matches for: %s" regexp))
        (romacs-project--resume
         note window buffer
         (lambda ()
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
    (advice-add 'project--read-regexp :before #'romacs-project--warm-at-prompt)
    (advice-add 'project-find-file :around #'romacs-project-find-file)
    (advice-add 'project-files :around #'romacs-project--time-files)
    (advice-add 'project-find-regexp :around #'romacs-project-find-regexp)))

;; A worker's vc-git asks Git its version once per session; its zygote
;; asks for all its children.  (Primitives only: this file also loads in
;; startups without subr.el.)
(when (and (boundp 'isolate-zygote-warm-functions)
           (not (assq 'vc-git isolate-zygote-warm-functions)))
  (setq isolate-zygote-warm-functions
        (cons '(vc-git . vc-git--program-version) isolate-zygote-warm-functions)))

(provide 'romacs-project)
;;; romacs-project.el ends here
