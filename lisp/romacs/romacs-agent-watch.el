;;; romacs-agent-watch.el --- Tell a task when the units it waits on end -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A delegated task that starts a long job as a systemd --user unit (a
;; check, a build) registers the unit with romacs_task_watch and ends its
;; turn.  A task may watch a file too, a section of it (a queue's "To
;; decide"): entries new under it are sent to it, each known by its first
;; word (an id), so a line rewritten (its age) is not new.  When the unit ends, the task is sent a short message: the unit,
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
  "A task's id to its watches, oldest first, each a plist: (:unit UNIT
:output FILE :since SECONDS), or (:file FILE :section HEADING :seen KEYS),
KEYS the first words of the entries already there.")

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
         file (vconcat (mapcar (lambda (w)
                                 (if (plist-get w :file)
                                     (romacs-mcp--object "file" (plist-get w :file)
                                                         "section" (or (plist-get w :section) :null)
                                                         "seen" (vconcat (plist-get w :seen)))
                                   (romacs-mcp--object "unit" (plist-get w :unit)
                                                       "output" (or (plist-get w :output) :null)
                                                       "since" (plist-get w :since))))
                               watches)))
      (when (file-exists-p file) (delete-file file)))))

(defun romacs-agent-watch--load (session)
  "SESSION's watches kept in its directory, known again."
  (let ((kept (romacs-agent--read-json (romacs-agent-watch--file session))))
    (when (vectorp kept)
      (puthash (romacs-agent-id session)
               (mapcar (lambda (w)
                         (if (gethash "file" w)
                             (list :file (gethash "file" w) :section (romacs-agent--value (gethash "section" w))
                                   :seen (append (gethash "seen" w) nil))
                           (list :unit (gethash "unit" w)
                                 :output (romacs-agent--value (gethash "output" w))
                                 :since (gethash "since" w))))
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
            ((string-match-p "\\`\\(?:[^ ]+: \\)?Stopped " line)
             (setq status (or status "stopped")))
            ;; A unit that ends well records no exit: the user manager
            ;; logs only what it consumed, and a collected unit leaves
            ;; no state to ask systemd for.
            ((string-match-p "Deactivated successfully\\|: Consumed .* CPU time" line)
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

;;; Watching a file

(defun romacs-agent-watch--entries (file section)
  "FILE's entries under the heading SECTION (anywhere, when nil): each
non-empty line not a heading, as (KEY . LINE), KEY its first word after a
list marker; nil when FILE cannot be read."
  (when (file-readable-p file)
    (with-temp-buffer
      (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
      (let ((in (null section)) (entries nil))
        (dolist (line (split-string (buffer-string) "\n"))
          (cond ((string-match "\\`#+[ \t]+\\(.*\\)" line)
                 (setq in (or (null section) (equal (string-trim (match-string 1 line)) section))))
                ((and in (string-match "\\`[ \t]*\\(?:[-*+][ \t]+\\)?\\([^ \t]+\\)" line))
                 (push (cons (match-string 1 line) (string-trim line)) entries))))
        (nreverse entries)))))

(defun romacs-agent-watch--check-file (id watch)
  "Tell task ID the entries new under WATCH's section since it last
looked, and remember what is there now (an entry gone and back is new)."
  (let* ((entries (romacs-agent-watch--entries (plist-get watch :file) (plist-get watch :section)))
         (new (cl-remove-if (lambda (entry) (member (car entry) (plist-get watch :seen))) entries))
         (task (romacs-agent-get id)))
    ;; Told first: what could not be told is not yet seen.
    (when new
      (romacs-agent-tools--tell
       task (format "New under %s in %s (romacs_task_watch):\n%s"
                    (if (plist-get watch :section) (format "\"%s\"" (plist-get watch :section)) "its lines")
                    (plist-get watch :file) (mapconcat #'cdr new "\n"))))
    (unless (equal (mapcar #'car entries) (plist-get watch :seen))
      (plist-put watch :seen (mapcar #'car entries))
      (romacs-agent-watch--save task))))

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
  "Look at every watched file, and ask systemd about every watched unit
at once; report what changed."
  (let* ((all (romacs-agent-watch--all))
         (watches (cl-remove-if-not (lambda (pair) (plist-get (cdr pair) :unit)) all)))
    (romacs-agent-watch--schedule)
    (pcase-dolist (`(,id . ,watch) all)
      (when (plist-get watch :file)
        ;; One file's failure (its task's harness) is no other's.
        (condition-case err (romacs-agent-watch--check-file id watch)
          (error (message "romacs_task_watch: %s: %s" (plist-get watch :file) (error-message-string err))))))
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

(defun romacs-agent-watch--same-p (a b)
  "Whether watches A and B watch the same: a unit, or a file's section."
  (if (plist-get a :unit) (equal (plist-get a :unit) (plist-get b :unit))
    (and (equal (plist-get a :file) (plist-get b :file)) (equal (plist-get a :section) (plist-get b :section)))))

(defun romacs-agent-watch--named (watch)
  (if (plist-get watch :unit) (plist-get watch :unit)
    (concat (plist-get watch :file) (if (plist-get watch :section) (concat " § " (plist-get watch :section)) ""))))

(defun romacs-agent-watch--described (arguments task)
  "The watch ARGUMENTS describe for TASK: a unit, or a readable file and
its section, its entries now already seen; with stop, only its name (a
watched file may be gone)."
  (let ((unit (gethash "unit" arguments)) (file (gethash "file" arguments))
        (stop (eq (gethash "stop" arguments) t)))
    (cond ((and unit file) (user-error "Watch a unit or a file, not both"))
          (unit (list :unit (romacs-agent-watch--unit-name unit)
                      :output (and (gethash "output" arguments)
                                   (expand-file-name (gethash "output" arguments) (romacs-agent-cwd task)))
                      :since (float-time)))
          (file (let ((file (expand-file-name file (romacs-agent-cwd task))) (section (gethash "section" arguments)))
                  (unless (or stop (file-readable-p file)) (user-error "%s cannot be read" file))
                  (list :file file :section section
                        :seen (and (not stop) (mapcar #'car (romacs-agent-watch--entries file section))))))
          (t (user-error "Name a unit or a file to watch")))))

(defun romacs-agent-watch--watch (arguments)
  "Watch, or with stop no longer watch, ARGUMENTS' unit or file for the
task taskId (the caller's own by default), which the caller is or
delegated; a unit not running now is refused."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (id (or (gethash "taskId" arguments) caller (user-error "taskId is required: you are no task")))
         (task (or (romacs-agent-get id) (user-error "No task %s" id)))
         (watch (progn (unless (equal id caller) (romacs-agent-tools--own task caller "watch for it"))
                       (romacs-agent-watch--described arguments task)))
         (known (cl-find-if (lambda (w) (romacs-agent-watch--same-p w watch)) (gethash id romacs-agent-watch--watches))))
    (cond ((eq (gethash "stop" arguments) t)
           (when known (romacs-agent-watch--forget id known)))
          (known)
          (t (when (plist-get watch :unit)
               (let ((state (romacs-agent-watch--unit-state (plist-get watch :unit))))
                 (unless (romacs-agent-watch--running-p state)
                   (user-error "Unit %s is not running (%s): start it first, or read its end yourself"
                               (plist-get watch :unit) state))))
             (puthash id (append (gethash id romacs-agent-watch--watches) (list watch)) romacs-agent-watch--watches)
             (romacs-agent-watch--save task)
             (romacs-agent-watch--schedule)))
    (romacs-mcp--object
     "taskId" id
     "unit" (or (plist-get watch :unit) :null) "output" (or (plist-get watch :output) :null)
     "file" (or (plist-get watch :file) :null) "section" (or (plist-get watch :section) :null)
     "watching" (vconcat (mapcar #'romacs-agent-watch--named (gethash id romacs-agent-watch--watches)))
     "next" (cond ((eq (gethash "stop" arguments) t) :null)
                  ((plist-get watch :unit)
                   "End your turn now: when the unit ends, its status and the output's last lines come as your next message. Do not wait on it or poll it.")
                  (t "End your turn now: entries new in it come as your next message. Do not poll it.")))))

(romacs-mcp-register-tool
 "romacs_task_watch"
 "Watch a systemd --user unit (a check, a build) a task waits on, instead of holding a waiter: when it ends, the \
task (taskId, default your own session) is sent the unit, its exit status as systemd recorded it, and the last 20 \
lines of output (a file the job writes, relative to the task's directory), as its next message. Its parent is not \
woken for it; the task's own next end wakes its parent as usual. The unit must be running; taskId must be you or \
a task you delegated. Or watch a file: file (relative to the task's directory), section (a heading's text; \
default the whole file): entries new under it are sent to the task as they appear, each known by its first word \
(an id: a line rewritten is not new), until stopped: a queue's \"To decide\" section wakes you as it fills. stop \
true ends the watch named. Watches outlive a Romacs restart."
 (romacs-mcp--schema '(("instance" . "string") ("taskId" . "string") ("unit" . "string") ("output" . "string")
                       ("file" . "string") ("section" . "string") ("stop" . "boolean"))
                     nil)
 #'romacs-agent-watch--watch)

(provide 'romacs-agent-watch)
;;; romacs-agent-watch.el ends here
