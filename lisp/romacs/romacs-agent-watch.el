;;; romacs-agent-watch.el --- Tell a task when the units it waits on end -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A delegated task that starts a long job as a systemd --user unit (a
;; check, a build) registers the unit with romacs_task_watch and ends its
;; turn.  When the unit ends, the task is sent a short message: the unit,
;; its exit status as systemd recorded it, and the last lines of the job's
;; output file.  Nobody holds a waiter, and the lead is not woken for it:
;; the task's own next end wakes its parent by its wake rule.
;;
;; One timer, running only while something is watched, asks systemd for
;; every watched unit's state in one `systemctl --user show' every
;; `romacs-agent-watch-interval' seconds.  A unit no longer active has
;; ended; its status is read from the user journal (`journalctl -u',
;; since the watch began), which keeps it after a --collect unit is gone.
;; Watches are kept in their task's directory: a restarted server goes on.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-tools)

(defcustom romacs-agent-watch-interval 15
  "Seconds between asking systemd about the units tasks watch."
  :type 'number :group 'romacs-agent)

(defconst romacs-agent-watch--lines 20
  "How many of a job's last output lines its task is sent.")

(defvar romacs-agent-watch--watches (make-hash-table :test #'equal)
  "A task's id to its watches, each a plist (:unit UNIT :output FILE
:since SECONDS), oldest first.")

(defvar romacs-agent-watch--timer nil)

(defvar romacs-agent-watch--asking nil
  "The `systemctl show' process of the tick under way, or nil.")

;;; Records

(defun romacs-agent-watch--file (session)
  (expand-file-name "watches.json" (romacs-agent-dir session)))

(defun romacs-agent-watch--save (session)
  "Keep SESSION's watches in its directory, or none."
  (let ((watches (gethash (romacs-agent-id session) romacs-agent-watch--watches))
        (file (romacs-agent-watch--file session)))
    (if watches
        (romacs-agent--write-json
         file (vconcat (mapcar (lambda (w) (romacs-mcp--object "unit" (plist-get w :unit)
                                                               "output" (or (plist-get w :output) :null)
                                                               "since" (plist-get w :since)))
                               watches)))
      (when (file-exists-p file) (delete-file file)))))

(defun romacs-agent-watch--load (session)
  "SESSION's watches kept in its directory, known again."
  (let ((kept (romacs-agent--read-json (romacs-agent-watch--file session))))
    (when (vectorp kept)
      (puthash (romacs-agent-id session)
               (mapcar (lambda (w) (list :unit (gethash "unit" w)
                                         :output (romacs-agent--value (gethash "output" w))
                                         :since (gethash "since" w)))
                       kept)
               romacs-agent-watch--watches))))

;;; Reading systemd

(defun romacs-agent-watch--states (text)
  "TEXT of `systemctl show -p Id,ActiveState' over units, as a table of
each unit's id to its active state."
  (let ((states (make-hash-table :test #'equal)) (id nil))
    (with-temp-buffer
      (insert text)
      (goto-char (point-min))
      ;; A unit's block starts with its Id; its state follows in it.
      (while (re-search-forward "^\\(Id\\|ActiveState\\)=\\(.*\\)$" nil t)
        (if (equal (match-string 1) "Id")
            (setq id (match-string 2))
          (when id (puthash id (match-string 2) states)))))
    states))

(defun romacs-agent-watch--unit-name (unit)
  "UNIT as systemd names it: a service when it names no type."
  (if (string-match-p "\\.[a-z]+\\'" unit) unit (concat unit ".service")))

(defun romacs-agent-watch--running-p (state)
  (member state '("active" "activating" "deactivating" "reloading" "refreshing")))

(defun romacs-agent-watch--ended (watches states)
  "The WATCHES, (TASK-ID . WATCH) pairs, whose unit STATES says is no
longer running (a collected unit is inactive); one it does not name
\(systemd did not answer) is still waited on."
  (cl-remove-if-not (lambda (pair)
                      (let ((state (gethash (romacs-agent-watch--unit-name (plist-get (cdr pair) :unit)) states)))
                        (and state (not (romacs-agent-watch--running-p state)))))
                    watches))

(defun romacs-agent-watch--status (journal)
  "The exit a unit's JOURNAL lines (`journalctl -o cat') last record."
  (let ((status nil))
    (dolist (line (split-string journal "\n" t))
      (cond ((string-match "Main process exited, code=\\([a-z]+\\), status=\\([^ ]+\\)" line)
             (setq status (format "%s, status=%s" (match-string 1 line) (match-string 2 line))))
            ((string-match-p "Deactivated successfully" line)
             (setq status (or status "exited, status=0/SUCCESS")))
            ((string-match "Failed with result '\\([^']+\\)'" line)
             (setq status (format "%s (failed: %s)" (or status "no exit recorded") (match-string 1 line))))))
    (or status "unknown: the journal recorded no end")))

(defun romacs-agent-watch--tail (file)
  "FILE's last `romacs-agent-watch--lines' lines, or why there are none."
  (if (not (file-readable-p file)) (format "(%s cannot be read)" file)
    (with-temp-buffer
      (let* ((size (file-attribute-size (file-attributes file)))
             (coding-system-for-read 'utf-8))
        (insert-file-contents file nil (max 0 (- size 16384)) size))
      (goto-char (point-max))
      (skip-chars-backward "\n")
      (let ((end (point)))
        (forward-line (- 1 romacs-agent-watch--lines))
        (buffer-substring-no-properties (line-beginning-position) end)))))

(defun romacs-agent-watch--message (watch status)
  "What a task is told when WATCH's unit ended with STATUS."
  (concat (format "Unit %s ended: %s (romacs_task_watch)." (plist-get watch :unit) status)
          (when (plist-get watch :output)
            (format "\nLast lines of %s:\n%s" (plist-get watch :output)
                    (romacs-agent-watch--tail (plist-get watch :output))))))

;;; The reactor

(defun romacs-agent-watch--all ()
  "Every watch, (TASK-ID . WATCH); a cancelled or forgotten task's dropped."
  (let ((all nil))
    (maphash (lambda (id watches)
               (let ((task (romacs-agent-get id)))
                 (if (or (null task) (romacs-agent-cancelled task))
                     (progn (remhash id romacs-agent-watch--watches) (when task (romacs-agent-watch--save task)))
                   (dolist (watch watches) (push (cons id watch) all)))))
             romacs-agent-watch--watches)
    all))

(defun romacs-agent-watch--schedule ()
  "Run the timer while something is watched, and only then."
  (if (> (hash-table-count romacs-agent-watch--watches) 0)
      (unless romacs-agent-watch--timer
        (setq romacs-agent-watch--timer
              (run-at-time romacs-agent-watch-interval romacs-agent-watch-interval #'romacs-agent-watch--tick)))
    (when romacs-agent-watch--timer
      (cancel-timer romacs-agent-watch--timer)
      (setq romacs-agent-watch--timer nil))))

(defun romacs-agent-watch--forget (id watch)
  (let ((task (romacs-agent-get id)))
    (puthash id (delq watch (gethash id romacs-agent-watch--watches)) romacs-agent-watch--watches)
    (unless (gethash id romacs-agent-watch--watches) (remhash id romacs-agent-watch--watches))
    (when task (romacs-agent-watch--save task))
    (romacs-agent-watch--schedule)))

(defun romacs-agent-watch--output (program args then)
  "Run PROGRAM with ARGS off the command loop; call THEN with its output."
  (let ((buffer (generate-new-buffer " *romacs-agent-watch*")))
    (make-process :name "romacs-agent-watch" :buffer buffer :command (cons program args)
                  :connection-type 'pipe :noquery t :stderr null-device
                  :sentinel (lambda (process _event)
                              (unless (process-live-p process)
                                (unwind-protect
                                    (funcall then (with-current-buffer buffer (buffer-string)))
                                  (kill-buffer buffer)))))))

(defun romacs-agent-watch--report (id watch)
  "Tell task ID that WATCH's unit ended, with its status from the journal."
  (romacs-agent-watch--output
   "journalctl" (list "--user" "-u" (plist-get watch :unit) "-o" "cat" "--no-pager"
                      "--since" (format "@%d" (floor (plist-get watch :since))))
   (lambda (journal)
     (when-let* ((task (romacs-agent-get id))
                 ((memq watch (gethash id romacs-agent-watch--watches))))
       (romacs-agent-watch--forget id watch)
       (unless (romacs-agent-cancelled task)
         (romacs-agent-tools--tell
          task (romacs-agent-watch--message watch (romacs-agent-watch--status journal))))))))

(defun romacs-agent-watch--tick ()
  "Ask systemd about every watched unit at once; report the ended ones."
  (let ((watches (romacs-agent-watch--all)))
    (romacs-agent-watch--schedule)
    (when (and watches (not (process-live-p romacs-agent-watch--asking)))
      (setq romacs-agent-watch--asking
            (romacs-agent-watch--output
             "systemctl" (append '("--user" "show" "-p" "Id,ActiveState" "--")
                                 (delete-dups (mapcar (lambda (pair) (romacs-agent-watch--unit-name
                                                                      (plist-get (cdr pair) :unit)))
                                                      watches)))
             (lambda (text)
               (pcase-dolist (`(,id . ,watch) (romacs-agent-watch--ended watches (romacs-agent-watch--states text)))
                 (romacs-agent-watch--report id watch))))))))

(defun romacs-agent-watch--restore ()
  "After a restart: the watches recorded in every session's directory."
  (dolist (session (romacs-agent-sessions)) (romacs-agent-watch--load session))
  (romacs-agent-watch--schedule))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-watch--restore)

;;; The tool

(defun romacs-agent-watch--unit-state (unit)
  "UNIT's active state now, asked of systemd."
  (with-temp-buffer
    (call-process "systemctl" nil t nil "--user" "show" "-p" "ActiveState" "--value" "--" unit)
    (string-trim (buffer-string))))

(defun romacs-agent-watch--watch (arguments)
  "Watch ARGUMENTS' unit for the task taskId (the caller's own by default),
which the caller is or delegated; refused for a unit not running now."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (id (or (gethash "taskId" arguments) caller (user-error "taskId is required: you are no task")))
         (task (or (romacs-agent-get id) (user-error "No task %s" id)))
         (unit (romacs-agent-watch--unit-name (gethash "unit" arguments)))
         (output (and (gethash "output" arguments) (expand-file-name (gethash "output" arguments)
                                                                     (romacs-agent-cwd task)))))
    (unless (equal id caller)
      (romacs-agent-tools--own task caller "watch units for it"))
    (let ((state (romacs-agent-watch--unit-state unit)))
      (unless (romacs-agent-watch--running-p state)
        (user-error "Unit %s is not running (%s): start it first, or read its end yourself" unit state)))
    (unless (cl-find unit (gethash id romacs-agent-watch--watches) :key (lambda (w) (plist-get w :unit)) :test #'equal)
      (puthash id (append (gethash id romacs-agent-watch--watches)
                          (list (list :unit unit :output output :since (float-time))))
               romacs-agent-watch--watches)
      (romacs-agent-watch--save task)
      (romacs-agent-watch--schedule))
    (romacs-mcp--object
     "taskId" id "unit" unit "output" (or output :null)
     "watching" (vconcat (mapcar (lambda (w) (plist-get w :unit)) (gethash id romacs-agent-watch--watches)))
     "next" "End your turn now: when the unit ends, its status and the output's last lines come as your next message. Do not wait on it or poll it.")))

(romacs-mcp-register-tool
 "romacs_task_watch"
 "Watch a systemd --user unit (a check, a build) a task waits on, instead of holding a waiter: when it ends, the \
task (taskId, default your own session) is sent the unit, its exit status as systemd recorded it, and the last 20 \
lines of output (a file the job writes, relative to the task's directory), as its next message. Its parent is not \
woken for it; the task's own next end wakes its parent as usual. The unit must be running; taskId must be you or \
a task you delegated. Watches outlive a Romacs restart."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string") ("unit" . "string") ("output" . "string"))
                     '("unit"))
 #'romacs-agent-watch--watch)

(provide 'romacs-agent-watch)
;;; romacs-agent-watch.el ends here
