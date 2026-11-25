;;; romacs-agent-watch.el --- Tell a task when the units it waits on end -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A delegated task that starts a long job as a systemd --user unit (a
;; check, a build) registers the unit with romacs_task_watch and ends its
;; turn.  A task may watch a file too, a section of it (a queue's "To
;; decide"): entries new under it are sent to it, each known by its first
;; word (an id), so a line rewritten (its age) is not new.  When the unit ends, the task is sent a short message: the unit,
;; its exit status from the unit's exit record, and the last lines of the
;; job's output file.  Nobody holds a waiter, and the lead is not woken for it:
;; the task's own next end wakes its parent by its wake rule.
;;
;; One timer, running only while something is watched, asks systemd for
;; every watched unit's state in one `systemctl --user show' every
;; `romacs-agent-watch-interval' seconds.  A unit no longer active has
;; ended, as has an invocation that systemd has replaced.  An ExecStopPost
;; hook may record its exit in $XDG_RUNTIME_DIR/unit-exit/UNIT; the record
;; must name the invocation the watch registered.
;; Watches are kept in their task's directory: a restarted server goes on.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'romacs-agent)
(require 'romacs-agent-tools)
(require 'romacs-agent-watch-exit)

(defcustom romacs-agent-watch-interval 15
  "Seconds between asking systemd about the units tasks watch."
  :type 'number :group 'romacs-agent)

(defconst romacs-agent-watch--lines 20
  "How many of a job's last output lines its task is sent.")

(defvar romacs-agent-watch--watches (make-hash-table :test #'equal)
  "A task's id to its watches, oldest first, each a plist: (:unit UNIT
:output FILE :invocation ID), or (:file FILE :section HEADING :seen KEYS
:version VERSION).  KEYS are the first words of the entries already there.
VERSION records the file last checked here and is kept only in memory.")

(defvar romacs-agent-watch--timer nil)

(defvar romacs-agent-watch--asking nil
  "The `systemctl show' process of the tick under way, or nil.")

(defvar romacs-agent-watch--reporting nil
  "Unit watches whose notification is under way.")

(defun romacs-agent-watch--stopped-p (session)
  "Whether SESSION has been cancelled or explicitly stopped."
  (or (romacs-agent-cancelled session)
      (and (romacs-agent-stopped session) (not (romacs-agent-reaped-p session)))))

(defun romacs-agent-watch-unit (session)
  "Return SESSION's oldest watched unit, or nil if no unit is watched."
  (when-let* ((unit (seq-some (lambda (watch) (plist-get watch :unit))
                             (gethash (romacs-agent-id session) romacs-agent-watch--watches)))
              ((not (romacs-agent-watch--stopped-p session))))
    unit))

;;; Records

(defun romacs-agent-watch--file (session)
  (expand-file-name "watches.json" (romacs-agent-dir session)))

(defun romacs-agent-watch--write (session watches)
  "Keep WATCHES in SESSION's directory, or remove its watch file."
  (let ((file (romacs-agent-watch--file session)))
    ;; Marked before its file is written, unmarked after it goes: a stop
    ;; between leaves a mark too many, which a load sees through.
    (when watches (romacs-agent-mark (romacs-agent-id session) 'watches t))
    (if watches
        (romacs-agent--write-json
         file (vconcat (mapcar (lambda (w)
                                 (if (plist-get w :file)
                                     (romacs-mcp--object "file" (plist-get w :file)
                                                         "section" (or (plist-get w :section) :null)
                                                         "seen" (vconcat (plist-get w :seen)))
                                   (romacs-mcp--object "unit" (plist-get w :unit)
                                                       "output" (or (plist-get w :output) :null)
                                                       "invocation" (or (plist-get w :invocation) :null))))
                               watches)))
      (when (file-exists-p file) (delete-file file)))
    (unless watches
      (with-demoted-errors "romacs_task_watch: clearing the watch mark: %S"
        (romacs-agent-mark (romacs-agent-id session) 'watches nil)))))

(defun romacs-agent-watch--put (id watches)
  "Make WATCHES, nil for none, the task ID's: its summary's watched unit
follows (`romacs-agent-watch-unit')."
  (if watches (puthash id watches romacs-agent-watch--watches) (remhash id romacs-agent-watch--watches))
  (romacs-agent--summary-stale id))

(defun romacs-agent-watch--set (session watches &optional persist)
  "Replace SESSION's WATCHES and notify observers when they change.
With PERSIST, write the change before publishing and scheduling it."
  (let* ((id (romacs-agent-id session))
         (previous (gethash id romacs-agent-watch--watches)))
    (when persist (romacs-agent-watch--write session watches))
    (romacs-agent-watch--put id watches)
    (when persist (romacs-agent-watch--schedule))
    (unless (equal previous watches)
      (romacs-agent--run-hook 'romacs-agent-event-functions session '(:type watch-changed)))))

(defun romacs-agent-watch-stop (session)
  "Retire SESSION's watches when its user stops or cancels it."
  (when (or (gethash (romacs-agent-id session) romacs-agent-watch--watches)
            (and (romacs-agent-dir session) (file-exists-p (romacs-agent-watch--file session))))
    (romacs-agent-watch--set session nil t)))

(defun romacs-agent-watch--load (session)
  "SESSION's watches kept in its directory, known again."
  (if (romacs-agent-watch--stopped-p session)
      (romacs-agent-watch-stop session)
    (romacs-agent-watch--load-kept session)))

(defun romacs-agent-watch--load-kept (session)
  "Read SESSION's watches from disk."
  (let ((kept (and (romacs-agent-dir session)
                   (romacs-agent--read-json (romacs-agent-watch--file session)))))
    (when (vectorp kept)
      (romacs-agent-watch--set
       session
       (mapcar (lambda (w)
                 (if (gethash "file" w)
                     (list :file (gethash "file" w) :section (romacs-agent--value (gethash "section" w))
                           :seen (append (gethash "seen" w) nil) :version nil)
                   (list :unit (gethash "unit" w)
                         :output (romacs-agent--value (gethash "output" w))
                         :invocation (romacs-agent--value (gethash "invocation" w)))))
               kept)))))

;;; Reading systemd

(defconst romacs-agent-watch--show-properties "Id,ActiveState,InvocationID"
  "The systemctl properties used to identify a watched invocation.")

(defun romacs-agent-watch--states (text)
  "Parse systemctl output TEXT into a table of unit names to (STATE . INVOCATION).
An omitted or empty InvocationID is represented by nil."
  (let ((states (make-hash-table :test #'equal)))
    (dolist (block (split-string text "\n\n" t) states)
      (let* ((properties (mapcar (lambda (line) (split-string line "="))
                                 (split-string block "\n" t)))
             (unit (cadr (assoc "Id" properties)))
             (state (cadr (assoc "ActiveState" properties)))
             (invocation (cadr (assoc "InvocationID" properties))))
        (when (and unit state)
          (puthash unit (cons state (and invocation (not (string-empty-p invocation)) invocation))
                   states))))))

(defun romacs-agent-watch--unit-name (unit)
  "UNIT as systemd names it: a service when it names no type."
  (if (string-match-p "\\.[a-z]+\\'" unit) unit (concat unit ".service")))

(defun romacs-agent-watch--running-p (state)
  (member state '("active" "activating" "deactivating" "reloading" "refreshing")))

(defun romacs-agent-watch--ended (watches states)
  "Return (TASK-ID . WATCH) pairs in WATCHES whose invocation ended per STATES."
  (cl-remove-if-not
   (lambda (pair)
     (let* ((watch (cdr pair))
            (status (gethash (romacs-agent-watch--unit-name (plist-get watch :unit)) states))
            (expected (plist-get watch :invocation)))
       (and status
            (or (not (romacs-agent-watch--running-p (car status)))
                (and expected (cdr status) (not (equal expected (cdr status))))))))
   watches))

(defun romacs-agent-watch--exit-text (record)
  "Describe the unit's exit from RECORD, or say there is no record."
  (pcase record
    ('nil "no exit record")
    (`(,_ ,(and code (or "killed" "dumped")) ,status ,_)
     (format "%s, signal=%s" code status))
    (`(,result ,code ,status ,_)
     (format "%s, status=%s (%s)" code status result))))

(defun romacs-agent-watch--tail (file)
  "FILE's last `romacs-agent-watch--lines' lines, or why there are none.
A byte that is not UTF-8 (a character cut by the tail too) reads U+FFFD."
  (if (not (file-readable-p file)) (format "(%s cannot be read)" file)
    (with-temp-buffer
      (let* ((size (file-attribute-size (file-attributes file)))
             (coding-system-for-read 'utf-8))
        (insert-file-contents file nil (max 0 (- size 16384)) size))
      (goto-char (point-max))
      (skip-chars-backward "\n")
      (let ((end (point)))
        (forward-line (- 1 romacs-agent-watch--lines))
        (romacs-mcp--unicode (buffer-substring-no-properties (line-beginning-position) end))))))

(defun romacs-agent-watch--message (watch status)
  "What a task is told when WATCH's unit ended with STATUS."
  (concat (format "Unit %s ended: %s (romacs_task_watch)." (plist-get watch :unit) status)
          (when (plist-get watch :output)
            (format "\nLast lines of %s:\n%s" (plist-get watch :output)
                    (romacs-agent-watch--tail (plist-get watch :output))))))

;;; Watching a file

(defun romacs-agent-watch--file-lines (file)
  "Read FILE's lines as UTF-8, replacing raw bytes, or return nil if unreadable."
  (when (file-readable-p file)
    (with-temp-buffer
      (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
      (split-string (romacs-mcp--unicode (buffer-string)) "\n"))))

(defun romacs-agent-watch--section-entries (lines section)
  "The entries in LINES under SECTION, or anywhere if SECTION is nil.
Each is (KEY . LINE), with KEY the first word after a list marker."
  (let ((in (null section)) (entries nil))
    (dolist (line lines (nreverse entries))
      (cond ((string-match "\\`#+[ \t]+\\(.*\\)" line)
             (setq in (or (null section) (equal (string-trim (match-string 1 line)) section))))
            ((and in (string-match "\\`[ \t]*\\(?:[-*+][ \t]+\\)?\\([^ \t]+\\)" line))
             (push (cons (match-string 1 line) (string-trim line)) entries))))))

(defun romacs-agent-watch--entries (file section)
  "Read FILE's entries under SECTION, or all sections if SECTION is nil."
  (romacs-agent-watch--section-entries (romacs-agent-watch--file-lines file) section))

(defun romacs-agent-watch--file-version (file)
  "Return FILE's version, following a symlink to its target."
  (let ((attributes (file-attributes file)))
    (when (stringp (file-attribute-type attributes))
      (setq attributes (file-attributes (file-chase-links file))))
    (if attributes
        (list (file-attribute-file-identifier attributes)
              (file-attribute-size attributes)
              (file-attribute-modification-time attributes)
              (file-attribute-status-change-time attributes))
      :missing)))

(defun romacs-agent-watch--file-view (file)
  "Get FILE's (VERSION . ENTRIES-FUNCTION) for one poll.
The function reads the file once, on demand, and caches each section's entries."
  (let ((content nil) (sections nil))
    (cons (romacs-agent-watch--file-version file)
          (lambda (section)
            (let ((result (with-memoization content
                            (condition-case err (cons :ok (romacs-agent-watch--file-lines file))
                              (error err))))
                  (cache (with-memoization sections (make-hash-table :test #'equal))))
              (unless (eq (car result) :ok) (signal (car result) (cdr result)))
              (cdr (with-memoization (gethash section cache)
                     (cons :ok (romacs-agent-watch--section-entries (cdr result) section)))))))))

(defun romacs-agent-watch--new-entries (entries seen)
  "Return the ENTRIES with keys absent from SEEN, in file order."
  (let ((keys (make-hash-table :test #'equal)))
    (dolist (key seen) (puthash key t keys))
    (cl-remove-if (lambda (entry) (gethash (car entry) keys)) entries)))

(defun romacs-agent-watch--record-file (task watch keys version)
  "Keep KEYS and VERSION for TASK's WATCH if it is still registered."
  (let* ((id (romacs-agent-id task)) (watches (gethash id romacs-agent-watch--watches)))
    (when (and (memq watch watches) (eq task (gethash id romacs-agent--sessions)))
      (let* ((next (plist-put (copy-sequence watch) :seen keys))
             (updated (cl-substitute (plist-put next :version version) watch watches :test #'eq)))
        (romacs-agent-watch--write task updated)
        (romacs-agent-watch--put id updated)))))

(defun romacs-agent-watch--check-file (id watch view)
  "Tell task ID what is new in WATCH's section of VIEW, then record it."
  (let ((version (car view)))
    (unless (equal version (plist-get watch :version))
      (let* ((section (plist-get watch :section))
             (entries (funcall (cdr view) section))
             (keys (mapcar #'car entries))
             (seen (plist-get watch :seen)))
        (if (equal keys seen)
            (setf (plist-get watch :version) version)
          (when-let* ((task (romacs-agent-get id))
                      ((memq watch (gethash id romacs-agent-watch--watches))))
            ;; Told first: what could not be told is not yet seen.
            (when-let* ((new (romacs-agent-watch--new-entries entries seen)))
              (romacs-agent-tools--tell
               task (format "New under %s in %s (romacs_task_watch):\n%s"
                            (if section (format "\"%s\"" section) "its lines")
                            (plist-get watch :file) (mapconcat #'cdr new "\n"))))
            (romacs-agent-watch--record-file task watch keys version)))))))

;;; The reactor

(defun romacs-agent-watch--all ()
  "Every watch, (TASK-ID . WATCH); a cancelled or forgotten task's dropped."
  (let ((all nil))
    (maphash (lambda (id watches)
               (let ((task (romacs-agent-get id)))
                 (cond ((null task) (romacs-agent-watch--put id nil))
                       ((romacs-agent-watch--stopped-p task)
                        (with-demoted-errors "romacs_task_watch: clearing stopped task: %S"
                          (romacs-agent-watch-stop task)))
                       (t (dolist (watch watches) (push (cons id watch) all))))))
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

(defun romacs-agent-watch--forget (id watches)
  "Retire WATCHES of the task with ID."
  (if-let* ((task (romacs-agent-get id)))
      (romacs-agent-watch--set task (cl-remove-if (lambda (watch) (memq watch watches))
                                                (gethash id romacs-agent-watch--watches))
                               t)
    (romacs-agent-watch--put id nil)
    (romacs-agent-watch--schedule)))

(defun romacs-agent-watch--output (program args then)
  "Run PROGRAM with ARGS off the command loop; call THEN with its output
and its exit status."
  (let ((buffer (generate-new-buffer " *romacs-agent-watch*")))
    (make-process :name "romacs-agent-watch" :buffer buffer :command (cons program args)
                  :connection-type 'pipe :noquery t :stderr null-device
                  :sentinel (lambda (process _event)
                              (unless (process-live-p process)
                                (unwind-protect
                                    (funcall then (with-current-buffer buffer (buffer-string))
                                             (process-exit-status process))
                                  (kill-buffer buffer)))))))

(defun romacs-agent-watch--show (units then)
  "Ask systemd for UNITS and call THEN with their parsed states."
  (romacs-agent-watch--output
   "systemctl" (append (list "--user" "show" "-p" romacs-agent-watch--show-properties "--") units)
   (lambda (text _exit) (funcall then (romacs-agent-watch--states text)))))

(defun romacs-agent-watch--report (id watch)
  "Tell task ID that WATCH's unit ended and retire the watch once told.
Delivery is at-least-once under I/O failure."
  (when-let* ((task (romacs-agent-get id))
              ((not (romacs-agent-watch--stopped-p task)))
              ((memq watch (gethash id romacs-agent-watch--watches)))
              ((not (memq watch romacs-agent-watch--reporting))))
    (let* ((romacs-agent-watch--reporting (cons watch romacs-agent-watch--reporting))
           (unit (plist-get watch :unit))
           (record (romacs-agent-watch--exit-record unit (plist-get watch :invocation))))
      (romacs-agent-tools--tell task (romacs-agent-watch--message watch (romacs-agent-watch--exit-text record)))
      (romacs-agent-watch--forget id (list watch)))))

(defun romacs-agent-watch--check-files (watches)
  "Poll file WATCHES in order, sharing each file's observations."
  (let ((files (make-hash-table :test #'equal)))
    (pcase-dolist (`(,id . ,watch) watches)
      (when-let* ((file (plist-get watch :file)))
        (condition-case err
            (romacs-agent-watch--check-file
             id watch (with-memoization (gethash file files) (romacs-agent-watch--file-view file)))
          (error (message "romacs_task_watch: %s: %s" file (error-message-string err))))))))

(defun romacs-agent-watch--tick ()
  "Look at every watched file, and ask systemd about every watched unit
at once; report what changed."
  (let* ((all (romacs-agent-watch--all))
         (watches (cl-remove-if-not (lambda (pair) (plist-get (cdr pair) :unit)) all)))
    (romacs-agent-watch--schedule)
    (romacs-agent-watch--check-files all)
    (when (and watches (not (process-live-p romacs-agent-watch--asking)))
      (setq romacs-agent-watch--asking
            (romacs-agent-watch--show
             (delete-dups (mapcar (lambda (pair) (romacs-agent-watch--unit-name
                                                   (plist-get (cdr pair) :unit)))
                                  watches))
             (lambda (states)
               (pcase-dolist (`(,id . ,watch) (romacs-agent-watch--ended watches states))
                 (condition-case err (romacs-agent-watch--report id watch)
                   (error (message "romacs_task_watch: %s: %s" (plist-get watch :unit)
                                   (error-message-string err)))))))))))

(defun romacs-agent-watch--restore ()
  "After a restart: the watches recorded in every session's directory."
  (dolist (session (romacs-agent-sessions)) (romacs-agent-watch--load session))
  (romacs-agent-watch--schedule))

(add-hook 'romacs-agent-restored-hook #'romacs-agent-watch--restore)
;; A session with watches kept is loaded as a restore runs, so they go on.
(add-to-list 'romacs-agent-mark-kinds
             (cons 'watches (lambda (dir _manifest) (file-exists-p (expand-file-name "watches.json" dir)))))

;;; The tool

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
                                   (expand-file-name (gethash "output" arguments) (romacs-agent-cwd task)))))
          (file (let* ((file (expand-file-name file (romacs-agent-cwd task)))
                       (section (gethash "section" arguments))
                       (version (unless stop (romacs-agent-watch--file-version file))))
                  (unless (or stop (file-readable-p file)) (user-error "%s cannot be read" file))
                  (list :file file :section section :version version
                        :seen (and (not stop) (mapcar #'car (romacs-agent-watch--entries file section))))))
          (t (user-error "Name a unit or a file to watch")))))

(defun romacs-agent-watch--add (id task watch)
  "Watch WATCH for TASK, whose id is ID, and keep it in its directory."
  (romacs-agent-watch--set task (append (gethash id romacs-agent-watch--watches) (list watch)) t))

(defun romacs-agent-watch--dev-check (unit report invocation)
  "Watch check UNIT with output REPORT and launch ID INVOCATION for the caller.
Return non-nil when the task calling romacs_dev_check will be told."
  (when-let* ((id (romacs-agent-tools--caller)) (task (romacs-agent-get id)))
    (romacs-agent-watch--add id task (list :unit (romacs-agent-watch--unit-name unit)
                                          :output report :invocation invocation))
    t))

(setq romacs-mcp-dev-watch-function #'romacs-agent-watch--dev-check)

(defun romacs-agent-watch--update (task watch stop &optional status)
  "Update TASK's WATCH and return the tool reply.
STOP removes the watch.  STATUS is the unit's (STATE . INVOCATION)."
  (let* ((id (romacs-agent-id task))
         (unit (plist-get watch :unit))
         (known (cl-remove-if-not (lambda (w) (romacs-agent-watch--same-p w watch))
                                  (gethash id romacs-agent-watch--watches))))
    (cond
     (stop (when known (romacs-agent-watch--forget id known)))
     ((null unit) (unless known (romacs-agent-watch--add id task watch)))
     ((romacs-agent-watch--running-p (car status))
      (unless (cl-find (cdr status) known :test #'equal :key (lambda (w) (plist-get w :invocation)))
        (romacs-agent-watch--add id task (plist-put watch :invocation (cdr status)))))
     ((null known)
      (user-error "Unit %s is not running (%s): start it first, or read its end yourself" unit (car status))))
    (romacs-agent-watch--reply id watch stop)))

(defvar romacs-agent-watch--checking (make-hash-table :test #'equal)
  "(TASK-ID . UNIT) to the token of its registration still checking its
unit: a stop meanwhile drops it, and that registration installs nothing.")

(defun romacs-agent-watch--register-unit (task caller watch)
  "Defer registering TASK's unit WATCH until systemd replies to CALLER's
request, unless a stop of it comes meanwhile."
  (let* ((unit (plist-get watch :unit))
         (id (romacs-agent-id task))
         (key (cons id unit))
         (token (list 'checking))
         (retire (lambda ()
                   (when (eq (gethash key romacs-agent-watch--checking) token)
                     (remhash key romacs-agent-watch--checking)
                     t)))
         (states nil)
         (deadline (+ (float-time) 10))
         (process nil))
    (when (gethash key romacs-agent-watch--checking)
      (user-error "A watch on unit %s is already being checked" unit))
    (puthash key token romacs-agent-watch--checking)
    (condition-case err
        (setq process (romacs-agent-watch--show
                       (list unit) (lambda (result)
                                     (setq states result)
                                     (romacs-mcp-deferred-wake))))
      ((error quit)
       (funcall retire)
       (signal (car err) (cdr err))))
    (romacs-mcp-defer
     (lambda ()
       (cond (states
              (unless (funcall retire)
                (user-error "The watch on unit %s was stopped before it started" unit))
              (unless (and (eq (gethash id romacs-agent--sessions) task) (not (romacs-agent-watch--stopped-p task)))
                (user-error "Task %s stopped or disappeared before its watch could start" id))
              (unless (equal id caller) (romacs-agent-tools--own task caller "watch for it"))
              (list (romacs-agent-watch--update task watch nil (gethash unit states))))
             ((>= (float-time) deadline)
              (funcall retire)
              (when (process-live-p process) (delete-process process))
              (user-error "Timed out checking unit %s" unit))))
     deadline
     (lambda ()
       (funcall retire)
       (when (process-live-p process) (delete-process process))))))

(defun romacs-agent-watch--watch (arguments)
  "Watch, or with stop no longer watch, ARGUMENTS' unit or file for the
task taskId (the caller's own by default), which the caller is or
delegated; a unit not running now is refused."
  (romacs-mcp--instance arguments)
  (let* ((caller (romacs-agent-tools--caller))
         (task (romacs-agent-tools--known
                (or (gethash "taskId" arguments) caller (user-error "taskId is required: you are no task"))))
         (id (romacs-agent-id task))
         (watch (progn (unless (equal id caller) (romacs-agent-tools--own task caller "watch for it"))
                       (romacs-agent-watch--described arguments task)))
         (stop (eq (gethash "stop" arguments) t)))
    (when (and stop (plist-get watch :unit))
      (remhash (cons id (plist-get watch :unit)) romacs-agent-watch--checking))
    (if (and (plist-get watch :unit) (not stop))
        (romacs-agent-watch--register-unit task caller watch)
      (romacs-agent-watch--update task watch stop))))

(defun romacs-agent-watch--reply (id watch stop)
  "Describe the watches now kept for task ID after acting on WATCH.
STOP means a watch was removed."
  (romacs-mcp--object
   "taskId" id
   "unit" (or (plist-get watch :unit) :null) "output" (or (plist-get watch :output) :null)
   "file" (or (plist-get watch :file) :null) "section" (or (plist-get watch :section) :null)
   "watching" (vconcat (mapcar #'romacs-agent-watch--named (gethash id romacs-agent-watch--watches)))
   "next" (cond (stop :null)
                ((plist-get watch :unit)
                 "End your turn now: when the unit ends, its status and the output's last lines come as your next message. Do not wait on it or poll it.")
                (t "End your turn now: entries new in it come as your next message. Do not poll it."))))

(romacs-mcp-register-tool
 "romacs_task_watch"
 "Watch a systemd --user unit (a check, a build) a task waits on, instead of holding a waiter: when it ends, the \
task (taskId, default your own session) is sent the unit, its status from the matching invocation's record at \
$XDG_RUNTIME_DIR/unit-exit/UNIT (the full unit name), and the last 20 lines of output from a file the job writes, \
relative to the task's directory, as its next message. Its parent is not \
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
