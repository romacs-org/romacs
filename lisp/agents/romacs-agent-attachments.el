;;; romacs-agent-attachments.el --- Files an agent attaches to its reply  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; An agent attaches files or images to the reply it is writing
;; (romacs_attach): they are referenced where they are, never copied.
;; Each is journaled in the session's journal as an `attachments' note,
;; with its kind, size and sha256, and folds into an `attachments' event
;; in the turn, which the chat draws with the reply.  Two of them can be
;; marked a before-and-after pair.

;;; Code:

(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-agent)
(require 'romacs-attach)

(declare-function romacs-agent-tools--caller "romacs-agent-tools")

(defconst romacs-agent-attachments--limit 16
  "The most files one call attaches.")

(defconst romacs-agent-attachments--hash-limit (* 1024 1024 1024)
  "The largest file hashed: a larger one is attached with no sha256.")

(defconst romacs-agent-attachments--hash-seconds 20
  "How long hashing an attachment may take before it is given up.")

(defconst romacs-agent-attachments--seconds 60
  "How long one call may take, its files hashed and its note journaled,
before it is given up.")

(defun romacs-agent-attachments--job (request)
  "A call's job: what it owns until it is over, for MCP REQUEST (the
active request, to know a cancellation by)."
  (list :request request :processes nil :timers nil :waiter nil :over nil))

(defun romacs-agent-attachments--retire (job)
  "Let go of all JOB owns: its hashing processes, its timers, its waiter.
What comes late of them does nothing."
  (unless (plist-get job :over)
    (plist-put job :over t)
    (dolist (process (plist-get job :processes)) (when (process-live-p process) (delete-process process)))
    (mapc #'cancel-timer (plist-get job :timers))
    (when-let* ((waiter (plist-get job :waiter))) (remove-hook 'romacs-agent-event-functions waiter))))

(defun romacs-agent-attachments--gone-p (job)
  "Whether JOB's request was cancelled, or its caller's connection closed."
  (let ((request (plist-get job :request)))
    (and request (or (plist-get request :cancelled)
                     (not (romacs-mcp--live-p (plist-get request :peer) (plist-get request :generation)))))))

(defun romacs-agent-attachments--hash (job file size then)
  "Call THEN with FILE's sha256, or nil when it is larger than
`romacs-agent-attachments--hash-limit' (SIZE its size), `sha256sum'
cannot run, or hashing fails or takes too long; never once JOB is over.
Off the command loop, its output unescaped whatever the file's name."
  (if (> size romacs-agent-attachments--hash-limit) (funcall then nil)
    (let* ((output (generate-new-buffer " *romacs-attach-sha*" t))
           (done nil)
           (settle (lambda (sha)
                     (unless done
                       (setq done t)
                       (when (buffer-live-p output) (kill-buffer output))
                       (unless (plist-get job :over) (funcall then sha)))))
           (process (condition-case nil
                        (make-process
                         :name "romacs-attach-sha" :buffer output :noquery t :connection-type 'pipe
                         :command (list "sha256sum" "--zero" "--" file)
                         :sentinel (lambda (process _event)
                                     (unless (process-live-p process)
                                       (funcall settle
                                                (and (eql (process-exit-status process) 0) (buffer-live-p output)
                                                     (with-current-buffer output
                                                       (goto-char (point-min))
                                                       (and (looking-at "[0-9a-f]\\{64\\}") (match-string 0))))))))
                      (error nil))))
      (if (not process) (funcall settle nil)
        (plist-put job :processes (cons process (plist-get job :processes)))
        (plist-put job :timers (cons (run-at-time romacs-agent-attachments--hash-seconds nil
                                                  (lambda () (unless done (delete-process process) (funcall settle nil))))
                                     (plist-get job :timers)))))))

(defun romacs-agent-attachments--check (path)
  "PATH, an attachment's, when it names an absolute local regular file not
under /proc, /sys or /dev; else a `user-error'.  Its metadata is its
target's when it is a symbolic link."
  (unless (and (stringp path) (file-name-absolute-p path))
    (user-error "Attach a file by its absolute path: %S" path))
  (let ((target (file-truename (expand-file-name path))))
    (when (string-match-p "\\`/\\(?:proc\\|sys\\|dev\\)/" target)
      (user-error "Not a file to attach: %s" path))
    (romacs-attach-item target)))

(defun romacs-agent-attachments--alts (alt count)
  "ALT, a string for one file or an array of one per file, as COUNT captions."
  (cond ((null alt) (make-list count nil))
        ((stringp alt)
         (unless (= count 1) (user-error "alt for several files is an array, one per file"))
         (list alt))
        ((and (vectorp alt) (= (length alt) count) (cl-every (lambda (a) (or (stringp a) (eq a :null))) alt))
         (mapcar (lambda (a) (and (stringp a) a)) (append alt nil)))
        (t (user-error "alt is a string, or an array of one per file"))))

(defun romacs-agent-attachments--await (job session id then)
  "Call THEN with t once SESSION's attachments note ID is read back from
its journal, or with nil should its harness end first; JOB owns the
wait."
  (letrec ((waiter (lambda (seen event)
                     (when (eq seen session)
                       (pcase (plist-get event :type)
                         ('attachments (when (equal (plist-get event :id) id)
                                         (remove-hook 'romacs-agent-event-functions waiter)
                                         (funcall then t)))
                         ((or 'exit 'unstarted)
                          (remove-hook 'romacs-agent-event-functions waiter)
                          (funcall then nil)))))))
    (plist-put job :waiter waiter)
    (add-hook 'romacs-agent-event-functions waiter)))

(defun romacs-agent-attachments--tool (arguments)
  "Attach ARGUMENTS' paths to the calling session's reply: answered once
every file is hashed and the note is in its journal, its place in the
turn known."
  (romacs-mcp--instance arguments)
  (let* ((caller (or (when-let* ((id (romacs-agent-tools--caller))) (romacs-agent-get id))
                     (user-error "romacs_attach attaches to an agent session's reply: you are none")))
         (paths (append (gethash "paths" arguments) nil))
         (compare (append (gethash "compare" arguments) nil)))
    (unless (and paths (cl-every #'stringp paths)) (user-error "paths is a non-empty array of absolute paths"))
    (when (> (length paths) romacs-agent-attachments--limit)
      (user-error "At most %d files at once" romacs-agent-attachments--limit))
    (when compare
      (unless (and (= (length compare) 2) (cl-every (lambda (p) (member p paths)) compare))
        (user-error "compare is [before, after], two of paths")))
    (let* ((files (mapcar #'romacs-agent-attachments--check paths))
           (items (cl-mapcar (lambda (path file alt)
                               (romacs-mcp--object "path" path
                                                   "kind" (if (string-prefix-p "image/" (plist-get file :mime)) "image" "file")
                                                   "alt" (or alt :null) "size" (plist-get file :bytes) "sha" :null))
                             paths files (romacs-agent-attachments--alts (gethash "alt" arguments) (length paths))))
           (waiting (length items))
           (id (romacs-agent--uuid))
           (cell (list nil))
           (job (romacs-agent-attachments--job romacs-mcp--active))
           (finish (lambda (outcome)
                     (unless (car cell) (setcar cell outcome))
                     (romacs-agent-attachments--retire job))))
      ;; Its lifetime: given up when its request is, or after a while.
      (plist-put job :timers
                 (list (run-at-time romacs-agent-attachments--seconds nil
                                    (lambda () (funcall finish (list :error "Attaching took too long; try again"))))
                       (run-at-time 1 1 (lambda () (when (romacs-agent-attachments--gone-p job)
                                                     (funcall finish (list :error "cancelled")))))))
      ;; Its session's end, and its note read back, are watched from now:
      ;; an end while hashing gives it up too.
      (romacs-agent-attachments--await
       job caller id
       (lambda (journaled)
         (funcall finish (if journaled (list :done)
                           (list :error "Its harness ended before the attachments were journaled")))))
      (cl-mapc (lambda (item file)
                 (romacs-agent-attachments--hash
                  job (plist-get file :path) (plist-get file :bytes)
                  (lambda (sha)
                    (cond
                     ;; Its request gone (cancelled, its caller away): no further.
                     ((romacs-agent-attachments--gone-p job) (funcall finish (list :error "cancelled")))
                     ;; Hashed as it was when checked, or not at all.
                     ((not (equal (romacs-attach--identity (plist-get file :path)) (plist-get file :identity)))
                      (funcall finish (list :error (format "%s changed while it was attached: attach it again"
                                                           (gethash "path" item)))))
                     (t
                      (when sha (puthash "sha" sha item))
                      (when (zerop (cl-decf waiting))
                        (condition-case err
                            (progn (when (or (romacs-agent-attachments--gone-p job) (plist-get job :over))
                                     (error "cancelled"))
                                   (romacs-agent--note caller (romacs-mcp--object "type" "attachments" "id" id
                                                                                  "items" (vconcat items)
                                                                                  "compare" (if compare (vconcat compare) :null))))
                          (error (funcall finish (list :error (error-message-string err)))))))))))
               items files)
      (romacs-mcp-defer
       (lambda ()
         (pcase (car cell)
           ('nil nil)
           (`(:error ,why) (user-error "%s" why))
           (_ (list (romacs-mcp--object "attached" (vconcat items)
                                        "next" "They are shown with your reply; nothing more to do.")))))))))

(romacs-mcp-register-tool
 "romacs_attach"
 "Attach files or images to the reply you are writing, so the user sees them with it: paths (absolute, local \
files, referenced where they are, not copied), alt (a caption: a string for one file, an array of one per file), \
compare ([before, after], two of paths: shown as a before-and-after pair). Answers each file's kind (image or \
file), size and sha256."
 (romacs-mcp--object
  "type" "object"
  "properties" (romacs-mcp--object
                "instance" (romacs-mcp--object "type" "string")
                "paths" (romacs-mcp--object "type" "array" "items" (romacs-mcp--object "type" "string"))
                "alt" (romacs-mcp--object "description" "A string for one file, or an array of one per file.")
                "compare" (romacs-mcp--object "type" "array" "items" (romacs-mcp--object "type" "string")))
  "required" ["paths"] "additionalProperties" :false)
 #'romacs-agent-attachments--tool)

(provide 'romacs-agent-attachments)
;;; romacs-agent-attachments.el ends here
