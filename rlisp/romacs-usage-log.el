;;; romacs-usage-log.el --- Which commands a session runs, in a local log  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; With ROMACS_USAGE_LOG naming a file, an interactive session appends JSON
;; lines to it (a launcher can set it per session and a summariser
;; read it):
;;
;;   {"kind":"command","command":C,"mode":M,"n":N}
;;       how often command C ran in major mode M, written every
;;       `romacs-usage-log-interval' seconds and at `kill-emacs';
;;   {"kind":"command","command":C,"mode":M,"ms":MS}
;;       a command that took `romacs-usage-log-slow-ms' or more, from
;;       `pre-command-hook' to `post-command-hook' (a command that reads
;;       input itself counts the wait);
;;   {"kind":"error","command":C,"error":"(SYMBOL . MESSAGE)"}
;;       an error the command loop reported, through `command-error-function';
;;   {"kind":"session","event":"start"|"init-done"|"kill","seconds":S}
;;       S seconds after `before-init-time';
;;   {"kind":"slow","command":C,"mode":M,"n":N,"ms":TOTAL,"max":MAX}
;;       commands C in mode M whose command loop (from `pre-command-hook'
;;       to the end of `post-command-hook', its other functions too) took
;;       `romacs-usage-log-slow-loop-ms' or more: how many, their total
;;       and their longest, written with the counts.  The redisplay after
;;       a command has no hook to time it by, and is not counted.
;; An error record of a kind (its command and its error symbol) not yet
;; seen this session also has "backtrace": the names of the functions
;; its signal went through, innermost first (no arguments, no text).
;;
;; A server (`romacs --server NAME') whose environment names no log keeps
;; its own record as a launcher does for a session it starts:
;; `romacs-usage-log-sessions'/START-PID/ with usage.jsonl and a
;; session.json written at its start and its end (a server's unit fills an
;; abnormal end in, `romacs --server-exited'), the newest
;; `romacs-usage-log-sessions-kept' kept.
;;
;; It logs command symbols, major modes, error symbols and messages, never
;; buffer text, file names or arguments: a message keeps 200 characters,
;; and one with a file name in it (a slash) is only <file>.  Between
;; commands the log only counts and takes the time; its lines are written
;; together, at the interval and at `kill-emacs'.  Batch sessions never log.
;; A message can still hold text an error was given, such as a search's.

;;; Code:

(defvar romacs-usage-log-file nil
  "The file this session logs its commands to, or nil when it does not.")

(defvar romacs-usage-log-interval 60
  "Seconds between the log's writes.")

(defvar romacs-usage-log-slow-ms 100
  "Milliseconds from which a command is logged as slow.")

(defvar romacs-usage-log-slow-loop-ms 50
  "Milliseconds from which a command's loop counts in its kind's slow record.")

(defvar romacs-usage-log-sessions-kept 50
  "How many session records a server keeps.")

(defvar romacs-usage-log--slow (make-hash-table :test 'equal)
  "\"COMMAND MODE\" to [N TOTAL-MS MAX-MS], since the last write.")

(defvar romacs-usage-log--settling nil
  "The command whose loop is ending: [COMMAND MODE START], for the last
`post-command-hook' function.")

(defvar romacs-usage-log--traced (make-hash-table :test 'equal)
  "\"COMMAND SYMBOL\" kinds whose error record had a backtrace.")

(defvar romacs-usage-log--captured (make-hash-table :test 'equal)
  "\"COMMAND SYMBOL\" to when a backtrace was last taken for it: one a
second at most, for a kind whose record has had none.")

(defvar romacs-usage-log--capture nil
  "The running command's last backtrace taken: [KIND DATA FRAMES].")

(defvar romacs-usage-log--counts (make-hash-table :test 'eq)
  "Major mode to a table of command to count, since the last write.")

(defvar romacs-usage-log--pending nil
  "Lines not yet written, newest first.")

(defvar romacs-usage-log--open nil
  "The commands running, innermost first: [DEPTH START MODE] for each,
its `recursion-depth', its start by `float-time' and its major mode.")

(defvar romacs-usage-log--timer nil)

(defun romacs-usage-log--name (command)
  "COMMAND's name; a command that is no symbol is anonymous."
  (if (symbolp command) (symbol-name command) "anonymous"))

(defun romacs-usage-log--push (&rest plist)
  "Queue PLIST's line."
  (push (json-serialize plist) romacs-usage-log--pending))

(defun romacs-usage-log--pre-command ()
  ;; A backtrace belongs to the command whose signal took it.
  (setq romacs-usage-log--capture nil)
  (push (vector (recursion-depth) (float-time) major-mode) romacs-usage-log--open))

(defun romacs-usage-log--settled ()
  "The command's loop ends (the last `post-command-hook' function): count
it when slow."
  (when-let* ((settling romacs-usage-log--settling))
    (setq romacs-usage-log--settling nil)
    (let ((ms (* 1000 (- (float-time) (aref settling 2)))))
      (when (>= ms romacs-usage-log-slow-loop-ms)
        (let* ((key (concat (aref settling 0) " " (aref settling 1)))
               (slot (or (gethash key romacs-usage-log--slow)
                         (puthash key (vector 0 0 0) romacs-usage-log--slow))))
          (aset slot 0 (1+ (aref slot 0)))
          (aset slot 1 (+ (aref slot 1) ms))
          (aset slot 2 (max (aref slot 2) ms)))))))

(defun romacs-usage-log--settle (command mode start)
  "COMMAND's loop ends with the last `post-command-hook' function."
  (setq romacs-usage-log--settling (vector command mode start)))

(defun romacs-usage-log--post-command ()
  "Count the command ending at this depth, and log it if it was slow.
A command a minibuffer read ran is timed from the key that ended the read,
whose command the read's exit left open deeper; a recursive edit's first
post-command run, which ends no command of its depth, counts nothing."
  (let ((depth (recursion-depth)) (read-ended nil))
    (while (and romacs-usage-log--open (> (aref (car romacs-usage-log--open) 0) depth))
      (setq read-ended (or read-ended (aref (car romacs-usage-log--open) 1))
            romacs-usage-log--open (cdr romacs-usage-log--open)))
    (when (and romacs-usage-log--open (= (aref (car romacs-usage-log--open) 0) depth))
      (let* ((open (pop romacs-usage-log--open))
             (command (if (symbolp real-this-command) real-this-command 'anonymous))
             (mode (aref open 2))
             (table (or (gethash mode romacs-usage-log--counts)
                        (puthash mode (make-hash-table :test 'eq) romacs-usage-log--counts)))
             (ms (* 1000 (- (float-time) (max (aref open 1) (or read-ended 0))))))
        (puthash command (1+ (gethash command table 0)) table)
        (romacs-usage-log--settle (symbol-name command) (romacs-usage-log--name mode)
                                  (max (aref open 1) (or read-ended 0)))
        (when (>= ms romacs-usage-log-slow-ms)
          (romacs-usage-log--push :kind "command" :command (symbol-name command)
                                  :mode (romacs-usage-log--name mode) :ms (round ms)))))))

(defun romacs-usage-log--scrub (text)
  "TEXT in at most 200 characters, or <file> if it has a slash: no regexp
can tell where a file name in it starts or ends."
  (cond ((string-search "/" text) "<file>")
        ((> (length text) 200) (substring text 0 200))
        (t text)))

(defun romacs-usage-log--error-text (data)
  "The logged form of the error DATA: (SYMBOL . MESSAGE)."
  (format "(%s . %S)" (car-safe data)
          (romacs-usage-log--scrub (condition-case nil (error-message-string data) (error "")))))

(defun romacs-usage-log--signal (symbol data)
  "`signal-hook-function': take the function names the signal of SYMBOL
went through, for an error kind no record has had a backtrace for yet,
once a second at most for each kind (a command that signals and catches
in a loop pays once).  A kind signalled again within that second keeps
the first signal's frames."
  (when-let* (((symbolp symbol))
              ((not (memq symbol '(quit minibuffer-quit user-error))))
              ((memq 'error (get symbol 'error-conditions)))
              (kind (format "%s %s" (romacs-usage-log--name real-this-command) symbol))
              ((not (gethash kind romacs-usage-log--traced)))
              (now (float-time))
              ((> (- now (gethash kind romacs-usage-log--captured 0)) 1)))
    (puthash kind now romacs-usage-log--captured)
    (setq romacs-usage-log--capture
          (vector kind data (let ((frames nil))
                       (mapbacktrace (lambda (_evald function _args _flags)
                                       (when (and (< (length frames) 40)
                                                  (not (memq function '(mapbacktrace romacs-usage-log--signal))))
                                         (push (if (symbolp function) (symbol-name function) "anonymous") frames))))
                       (nreverse frames))))))

(defun romacs-usage-log--command-error (data _context _signal)
  "Log the command loop's error DATA, before `command-error-function' reports it.
The first of its kind carries the backtrace its signal left, if one was taken."
  (condition-case nil
      (let* ((command (romacs-usage-log--name real-this-command))
             (kind (format "%s %s" command (car-safe data)))
             (capture romacs-usage-log--capture)
             ;; Its own signal's: the same kind and data, this command.
             (frames (and capture (not (gethash kind romacs-usage-log--traced))
                          (equal (aref capture 0) kind) (eq (aref capture 1) (cdr-safe data))
                          (aref capture 2))))
        (setq romacs-usage-log--capture nil)
        (if (not frames)
            (romacs-usage-log--push :kind "error" :command command :error (romacs-usage-log--error-text data))
          (puthash kind t romacs-usage-log--traced)
          (remhash kind romacs-usage-log--captured)
          (romacs-usage-log--push :kind "error" :command command :error (romacs-usage-log--error-text data)
                                  :backtrace (vconcat frames))))
    (error nil)))

(defun romacs-usage-log--seconds ()
  (if before-init-time
      (/ (round (* 1000 (float-time (time-subtract nil before-init-time)))) 1000.0)
    0))

(defun romacs-usage-log--event (event)
  (romacs-usage-log--push :kind "session" :event event :seconds (romacs-usage-log--seconds)))

(defun romacs-usage-log-flush ()
  "Append the queued lines and the counts since the last write to the log."
  (when romacs-usage-log-file
    (let ((counts nil))
      (maphash (lambda (mode table)
                 (maphash (lambda (command n)
                            (push (json-serialize (list :kind "command" :command (symbol-name command)
                                                        :mode (romacs-usage-log--name mode) :n n))
                                  counts))
                          table))
               romacs-usage-log--counts)
      (clrhash romacs-usage-log--counts)
      (maphash (lambda (key slot)
                 (let ((space (string-search " " key)))
                   (push (json-serialize (list :kind "slow" :command (substring key 0 space)
                                               :mode (substring key (1+ space)) :n (aref slot 0)
                                               :ms (round (aref slot 1)) :max (round (aref slot 2))))
                         counts)))
               romacs-usage-log--slow)
      (clrhash romacs-usage-log--slow)
      (let ((lines (nconc (nreverse romacs-usage-log--pending) (nreverse counts))))
        (setq romacs-usage-log--pending nil)
        (when lines (romacs-usage-log--append lines))))))

(defun romacs-usage-log--append (lines)
  "Append LINES to the log; a log that cannot be written stops, saying so."
  (let ((coding-system-for-write 'utf-8-unix)
        (create-lockfiles nil)
        (write-region-inhibit-fsync t)
        (file-name-handler-alist nil))
    (condition-case err
        (write-region (mapconcat (lambda (line) (concat line "\n")) lines "")
                      nil romacs-usage-log-file t 'silent)
      (error
       (romacs-usage-log-stop)
       (message "Usage log stopped: %s" (error-message-string err))))))

(defun romacs-usage-log--init-done ()
  ;; After the init, so the reporting the init chose is the one kept.
  (add-function :before command-error-function #'romacs-usage-log--command-error)
  ;; Another's hook stays: backtraces are only taken where none is set.
  (unless signal-hook-function (setq signal-hook-function #'romacs-usage-log--signal))
  (romacs-usage-log--event "init-done"))

(defun romacs-usage-log--kill ()
  (romacs-usage-log--event "kill")
  (romacs-usage-log-flush))

(defun romacs-usage-log--enable (file)
  "Log this session's commands to FILE."
  (setq romacs-usage-log-file (expand-file-name file))
  (add-hook 'pre-command-hook #'romacs-usage-log--pre-command)
  (add-hook 'post-command-hook #'romacs-usage-log--post-command)
  ;; Last: a command's loop ends after every other function.
  (add-hook 'post-command-hook #'romacs-usage-log--settled 100)
  (add-hook 'after-init-hook #'romacs-usage-log--init-done 90)
  (add-hook 'kill-emacs-hook #'romacs-usage-log--kill 90)
  (setq romacs-usage-log--timer
        (run-with-timer romacs-usage-log-interval romacs-usage-log-interval #'romacs-usage-log-flush))
  (romacs-usage-log--event "start")
  (romacs-usage-log-flush))

(defun romacs-usage-log-stop ()
  "Stop logging this session's commands."
  (setq romacs-usage-log-file nil romacs-usage-log--pending nil romacs-usage-log--open nil)
  (clrhash romacs-usage-log--counts)
  (remove-hook 'pre-command-hook #'romacs-usage-log--pre-command)
  (remove-hook 'post-command-hook #'romacs-usage-log--post-command)
  (remove-hook 'post-command-hook #'romacs-usage-log--settled)
  (remove-hook 'after-init-hook #'romacs-usage-log--init-done)
  (remove-hook 'kill-emacs-hook #'romacs-usage-log--kill)
  (remove-function command-error-function #'romacs-usage-log--command-error)
  (when (eq signal-hook-function #'romacs-usage-log--signal) (setq signal-hook-function nil))
  (setq romacs-usage-log--settling nil)
  (clrhash romacs-usage-log--slow)
  (when romacs-usage-log--timer (cancel-timer romacs-usage-log--timer))
  (setq romacs-usage-log--timer nil))

(defun romacs-usage-log-start ()
  "Log this session's commands to the file ROMACS_USAGE_LOG names, if it
names one and the session is interactive."
  (let ((file (getenv "ROMACS_USAGE_LOG")))
    (when (and file (not (string-empty-p file)) (not noninteractive))
      (romacs-usage-log--enable file))))

(defun romacs-usage-log-sessions ()
  "Where session records are: the launcher's, STATE/romacs/sessions/."
  (expand-file-name "romacs/sessions/" (or (getenv "XDG_STATE_HOME") "~/.local/state")))

(defun romacs-usage-log--session-pointer (name)
  "The file naming server NAME's session record, for its unit's end."
  (expand-file-name (format "romacs/server-%s.session" name) (or (getenv "XDG_STATE_HOME") "~/.local/state")))

(defvar romacs-usage-log--session nil
  "This server's session record: (DIRECTORY . SESSION-PLIST).")

(defun romacs-usage-log--write-session (end)
  "Write the session record's session.json; END non-nil: at its end."
  (when-let* ((session romacs-usage-log--session))
    (let ((record (copy-sequence (cdr session))) (now (float-time)))
      (when end
        ;; No status: `kill-emacs-hook' cannot see the exit code; the
        ;; unit's end (`romacs --server-exited') fills how it ended in.
        (setq record (append record (list :end now :seconds (/ (round (* 1000 (- now (plist-get record :start)))) 1000.0)))))
      (let ((coding-system-for-write 'utf-8-unix) (file (expand-file-name "session.json" (car session))))
        (write-region (concat (json-serialize record :null-object :null :false-object :false) "\n") nil file nil 'silent)))))

(defun romacs-usage-log--build ()
  "The build this process runs: its runtime's build.json commit and binary hash."
  (let* ((root (or (getenv "ROMACS_RUNTIME_ROOT") invocation-directory))
         (file (and root (expand-file-name "build.json" root)))
         (build (and file (file-readable-p file)
                     (ignore-errors (with-temp-buffer
                                      (insert-file-contents file)
                                      (json-parse-buffer :object-type 'plist :null-object nil))))))
    (list :commit (or (plist-get build :commit) :null) :binary_sha256 (or (plist-get build :binary_sha256) :null))))

(defun romacs-usage-log--prune (root kept)
  "Remove the session records in ROOT beyond the newest KEPT, by their
names (START-PID); a live process's record (another server's) stays."
  (dolist (stale (nthcdr kept (sort (directory-files root t "\\`[0-9]\\{8\\}T[0-9]\\{6\\}-[0-9]+\\'") #'string>)))
    (let ((pid (string-to-number (substring (file-name-nondirectory stale) 16))))
      (unless (or (= pid (emacs-pid)) (process-attributes pid))
        (ignore-errors (delete-directory stale t))))))

(defun romacs-usage-log-server-session (name)
  "Keep server NAME's own session record, as a launcher keeps a session's,
when the environment names no log and the session is interactive: a new
directory under `romacs-usage-log-sessions' (the oldest beyond
`romacs-usage-log-sessions-kept' removed), its usage.jsonl the log, its
session.json written now and at the end."
  (when (and (not noninteractive) (not romacs-usage-log-file)
             (let ((file (getenv "ROMACS_USAGE_LOG"))) (or (null file) (string-empty-p file))))
    (let* ((root (romacs-usage-log-sessions))
           (directory (expand-file-name (format "%s-%d" (format-time-string "%Y%m%dT%H%M%S") (emacs-pid)) root)))
      (make-directory root t)
      (set-file-modes root #o700)
      (make-directory directory t)
      (set-file-modes directory #o700)
      (romacs-usage-log--prune root romacs-usage-log-sessions-kept)
      (setq romacs-usage-log--session
            (cons directory (list :pid (emacs-pid) :server name
                                  :runtime (or (getenv "ROMACS_RUNTIME_ROOT") invocation-directory)
                                  :build (romacs-usage-log--build) :args (vconcat command-line-args)
                                  :start (float-time))))
      (romacs-usage-log--write-session nil)
      ;; The pointer names this run (systemd's invocation, which the unit's
      ;; ExecStopPost shares): an end is filled into its own record only.
      (let ((coding-system-for-write 'utf-8-unix))
        (write-region (json-serialize (list :directory directory :pid (emacs-pid)
                                            :invocation (or (getenv "INVOCATION_ID") :null))
                                      :null-object :null)
                      nil (romacs-usage-log--session-pointer name) nil 'silent))
      (add-hook 'kill-emacs-hook (lambda () (romacs-usage-log--write-session t)) 95)
      (romacs-usage-log--enable (expand-file-name "usage.jsonl" directory))
      ;; Started after the init: its reporting is set already.
      (when after-init-time
        (remove-hook 'after-init-hook #'romacs-usage-log--init-done)
        (romacs-usage-log--init-done))
      directory)))

(provide 'romacs-usage-log)
;;; romacs-usage-log.el ends here
