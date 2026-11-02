;;; romacs-precompile.el --- Compile likely libraries while the session is idle  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A compiled load runs a compiled-Lisp artifact where GNU would load an
;; `.elc' (crates/romacs-vm/src/compiled_lisp).  A load whose file has
;; none compiles it first, synchronously.  So that a first `require' finds
;; its artifact made already, an interactive session compiles ahead while
;; it is idle: the files compiled loads used before, then every file of the
;; installation's Lisp trees, as GNU's `make' compiles its `.elc' files
;; (`romacs--compiled-lisp-candidates').
;;
;; One compile process runs at a time, at the idle CPU and I/O priority,
;; and none starts while input is pending.  The session itself only starts
;; it and hears that it ended: no work happens in it between keys.  Not in
;; GNU (DIVERGENCES.md VM2-LOADPRECOMPILE).

;;; Code:

(defvar romacs-precompile t
  "Non-nil: compile likely libraries in the background while idle.")

(defvar romacs-precompile-idle-delay 2
  "Seconds of idleness before a background compile starts.")

(defvar romacs-precompile--queue t
  "The files still to look at, or t before the first look.")

(defvar romacs-precompile--process nil
  "The background compile process running, or nil.")

(defvar romacs-precompile--timer nil
  "The idle timer that starts background compiles.")

(defun romacs-precompile--next ()
  "The compile command of the next queued file still to compile, or nil.
Files that need no compile leave the queue on the way; looking stops as
soon as input arrives."
  (when (eq romacs-precompile--queue t)
    (setq romacs-precompile--queue (romacs--compiled-lisp-candidates)))
  (let (command)
    (while (and romacs-precompile--queue (not command) (not (input-pending-p)))
      (setq command (romacs--compiled-lisp-compile-command (pop romacs-precompile--queue))))
    command))

(defun romacs-precompile--step ()
  "Start the next background compile, when started, idle and none runs.
Once the queue is empty, stop."
  (when (and romacs-precompile romacs-precompile--timer (not romacs-precompile--process) (not (input-pending-p)))
    (let ((command (romacs-precompile--next)))
      (cond
       (command
        (let ((process-environment (nth 2 command)))
          (setq romacs-precompile--process
                (make-process :name "romacs-precompile"
                              :command (cons (car command) (nth 1 command))
                              :buffer nil :noquery t :connection-type 'pipe
                              :sentinel #'romacs-precompile--sentinel))))
       ((null romacs-precompile--queue) (romacs-precompile-stop))))))

(defun romacs-precompile--sentinel (process _event)
  "PROCESS, a background compile, ended: start the next at once while
the session has been idle `romacs-precompile-idle-delay' seconds, else at
its next idle time."
  (unless (process-live-p process)
    (setq romacs-precompile--process nil)
    (let ((idle (current-idle-time)))
      (when (and idle (>= (float-time idle) romacs-precompile-idle-delay))
        (run-at-time 0 nil #'romacs-precompile--step)))))

(defun romacs-precompile-start ()
  "Compile likely libraries in the background while idle, in an
interactive session that compiles Lisp."
  (when (and romacs-precompile (not noninteractive) (not romacs-precompile--timer)
             (fboundp 'romacs--compiled-lisp-candidates))
    (setq romacs-precompile--queue t
          romacs-precompile--timer
          (run-with-idle-timer romacs-precompile-idle-delay t #'romacs-precompile--step))))

(defun romacs-precompile-stop ()
  "Stop compiling in the background; a compile running finishes."
  (when romacs-precompile--timer
    (cancel-timer romacs-precompile--timer))
  (setq romacs-precompile--timer nil))

(add-hook 'emacs-startup-hook #'romacs-precompile-start)

(provide 'romacs-precompile)
;;; romacs-precompile.el ends here
