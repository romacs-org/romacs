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
;;   {"kind":"error","command":C,"mode":M,"error":"(SYMBOL . MESSAGE)"}
;;       an error the command loop reported, through `command-error-function',
;;       M the current buffer's major mode; "user":true besides when it is
;;       the user's own (`romacs-usage-log-user-error-p': a C-g, the
;;       buffer's end, a refusing command);
;;   {"kind":"refusal","class":MESSAGE,"mode":M,"n":N}
;;       how often a displayed window's layout question in major mode M
;;       was answered by approximation where the host's measurement refused
;;       (MESSAGE, `romacs--layout-refusals'), written with the counts;
;;   {"kind":"session","event":"start"|"init-done"|"kill","seconds":S}
;;       S seconds after `before-init-time';
;;   {"kind":"slow","command":C,"mode":M,"n":N,"ms":TOTAL,"redisplay_ms":R,"max":MAX}
;;       commands C in mode M whose command loop (from `pre-command-hook'
;;       to the end of `post-command-hook', its other functions too) and
;;       the redisplay after it (the first: the frame's snapshot, layout
;;       and paint's submission, as the host times it) took
;;       `romacs-usage-log-slow-loop-ms' or more together: how many, their
;;       loops' total, their redisplays' total and the longest of the two
;;       together, written with the counts.  A redisplay is charged to its
;;       command at the next command's start, or at a write after it ran.
;;   {"kind":"gaps","bounds_ms":[1,2,4,...,32768],"counts":[N0,N1,...]}
;;       how long the session waited from a command's end (its last
;;       `post-command-hook' function) to the next command's start, as
;;       counts in buckets that double: N0 gaps under 1 ms, N1 under 2 ms,
;;       and the last 32 s or more; written with the counts, when any.
;;       The idle windows a background task (a checkpoint, a fork) has.
;;   {"kind":"repeat","n":N,"bounds_ms":[1,...,32768],"after_press_ms":[N0,...],"longest_hold_ms":L}
;;       what the display saw of native key repeat since the last write: N
;;       repeats it let through, counted by how long after their key's
;;       press each came, in the gap buckets; L the longest a key was
;;       held, from the press it saw to the release it saw.  A key repeats
;;       until its release is seen, so a release seen late shows as repeats
;;       the user never held the key for.  Written with the counts, when a
;;       key was pressed and released or repeated.
;;   {"kind":"collection","collection":K,"pause_ms":MS,"t":T,"live_objects":O,"live_bytes_est":B,"late_objects":L,"met":M,"command":C}
;;       a garbage collection MS long, a float read to the microsecond, K
;;       `idle' (the idle pass's or a
;;       prompt's), `explicit' (`garbage-collect') or `automatic', written
;;       with the counts.  T is when it ended, as `float-time' reads; O and B
;;       what it left live in the private heap, B an estimate that does not
;;       walk and leaves out buffers' text and hash tables.  L is how far
;;       past its automatic trigger it began, in objects at the collector's
;;       charge (a float, as the trigger is): the allocation between falling
;;       due and the poll that took it.  M, when there is
;;       one, says who waited at most
;;       MS: "command", C being the command it ran in or at the end of;
;;       "key", a key press queued as it ended; or "redisplay", in a
;;       prompt's redisplay, where no key can be seen and any pressed waits.
;;       {"kind":"collection","dropped":N} counts those beyond what a
;;       session keeps between writes.
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

(defconst romacs-usage-log-user-conditions
  '(beginning-of-buffer end-of-buffer quit minibuffer-quit user-error text-read-only buffer-read-only
    mark-inactive)
  "Error conditions an ordinary action of the user's raises (moving past
the buffer's end, a C-g, a command refusing, typing into read-only text):
no fault of the editor's.  Reports set them apart.")

(defun romacs-usage-log-user-error-p (data)
  "Whether error DATA, (SYMBOL . DATA), is one of `romacs-usage-log-user-conditions'."
  (and (symbolp (car-safe data))
       (let ((conditions (get (car data) 'error-conditions)))
         (while (and conditions (not (memq (car conditions) romacs-usage-log-user-conditions)))
           (setq conditions (cdr conditions)))
         (and conditions t))))

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
  "\"COMMAND MODE\" to [N LOOP-MS MAX-MS REDISPLAY-MS], since the last write.")

(defvar romacs-usage-log--redisplayed nil
  "The command whose loop ended and whose redisplay is to come: [COMMAND
MODE LOOP-MS].")

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

(defconst romacs-usage-log--gap-bounds [1 2 4 8 16 32 64 128 256 512 1024 2048 4096 8192 16384 32768]
  "The gap histogram's bucket bounds, in milliseconds: a gap under a bound
counts in its bucket, one at least the last in the bucket after it.")

(defvar romacs-usage-log--gaps (make-vector 17 0)
  "Gaps between commands since the last write, by bucket
(`romacs-usage-log--gap-bounds').")

(defvar romacs-usage-log--idle-since nil
  "When the last command's loop ended, by `float-time', until the next
command starts.")

(defvar romacs-usage-log--timer nil)

(defun romacs-usage-log--name (command)
  "COMMAND's name; a command that is no symbol is anonymous."
  (if (symbolp command) (symbol-name command) "anonymous"))

(defun romacs-usage-log--push (&rest plist)
  "Queue PLIST's line."
  (push (json-serialize plist) romacs-usage-log--pending))

(defun romacs-usage-log--gap (ms)
  "Count a gap of MS milliseconds between two commands."
  (let ((bounds romacs-usage-log--gap-bounds) (bucket 0))
    (while (and (< bucket (length bounds)) (>= ms (aref bounds bucket)))
      (setq bucket (1+ bucket)))
    (aset romacs-usage-log--gaps bucket (1+ (aref romacs-usage-log--gaps bucket)))))

(defun romacs-usage-log--pre-command ()
  ;; A backtrace belongs to the command whose signal took it.
  (setq romacs-usage-log--capture nil)
  (when romacs-usage-log--idle-since
    (romacs-usage-log--gap (* 1000 (- (float-time) romacs-usage-log--idle-since)))
    (setq romacs-usage-log--idle-since nil))
  (romacs-usage-log--charge)
  (push (vector (recursion-depth) (float-time) major-mode) romacs-usage-log--open))

(defun romacs-usage-log--settled ()
  "The command's loop ends (the last `post-command-hook' function): time
it, and mark the host's redisplay for it."
  (when-let* ((settling romacs-usage-log--settling))
    (setq romacs-usage-log--settling nil)
    ;; One still waiting (a recursive edit's, ended with its caller):
    ;; no redisplay ran for it.
    (romacs-usage-log--charge)
    (aset settling 2 (* 1000 (- (float-time) (aref settling 2))))
    (setq romacs-usage-log--redisplayed settling
          romacs-usage-log--idle-since (float-time))
    (when (fboundp 'romacs--redisplay-since-mark) (romacs--redisplay-since-mark t))))

(defun romacs-usage-log--charge (&optional waiting)
  "Count the last command whose loop ended when its loop and its
redisplay (the first after its mark; none when the next command came
first, input pending) were slow together.  WAITING non-nil (a write
between commands): one whose redisplay has not run yet stays to be
charged."
  (let* ((done romacs-usage-log--redisplayed)
         (timed (fboundp 'romacs--redisplay-since-mark))
         (micros (and done timed (romacs--redisplay-since-mark))))
    ;; A host that times no redisplay has none to wait for.
    (when (and done (or micros (not waiting) (not timed)))
      (setq romacs-usage-log--redisplayed nil)
      (let* ((redisplay (if micros (/ micros 1000.0) 0))
             (loop (aref done 2)) (both (+ loop redisplay)))
        (when (>= both romacs-usage-log-slow-loop-ms)
        (let* ((key (concat (aref done 0) " " (aref done 1)))
               (slot (or (gethash key romacs-usage-log--slow)
                         (puthash key (vector 0 0 0 0) romacs-usage-log--slow))))
          (aset slot 0 (1+ (aref slot 0)))
          (aset slot 1 (+ (aref slot 1) loop))
          (aset slot 2 (max (aref slot 2) both))
          (aset slot 3 (+ (aref slot 3) redisplay))))))))

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
        (let ((record (list :kind "error" :command command :mode (romacs-usage-log--name major-mode)
                            :error (romacs-usage-log--error-text data))))
          ;; The user's own, apart: `romacs-usage-log-user-error-p'.
          (when (romacs-usage-log-user-error-p data) (setq record (append record (list :user t))))
          (when frames
            (puthash kind t romacs-usage-log--traced)
            (remhash kind romacs-usage-log--captured)
            (setq record (append record (list :backtrace (vconcat frames)))))
          (apply #'romacs-usage-log--push record)))
    (error nil)))

(defun romacs-usage-log--seconds ()
  (if before-init-time
      (/ (round (* 1000 (float-time (time-subtract nil before-init-time)))) 1000.0)
    0))

(defun romacs-usage-log--event (event)
  (romacs-usage-log--push :kind "session" :event event :seconds (romacs-usage-log--seconds)))

(defun romacs-usage-log--collections ()
  "The lines of the collections since the last call, oldest first."
  (when (fboundp 'romacs--take-collections)
    (let ((taken (romacs--take-collections)))
      (nconc
       (mapcar (lambda (record)
                 (let ((met (nth 2 record)) (command (nth 3 record)))
                   (json-serialize (append (list :kind "collection" :collection (symbol-name (nth 0 record)) :pause_ms (nth 1 record)
                                                 :t (nth 4 record) :live_objects (nth 5 record) :live_bytes_est (nth 6 record))
                                           (and (nth 7 record) (list :late_objects (nth 7 record)))
                                           (and met (list :met (symbol-name met)))
                                           (and command (list :command command))))))
               (cdr taken))
       (and (> (car taken) 0) (list (json-serialize (list :kind "collection" :dropped (car taken)))))))))

(defun romacs-usage-log--key-repeats ()
  "The `repeat' line for what the display saw of native key repeat since
the last write (`romacs--take-key-repeats'), or nil when it saw no key held
or repeated, or there is no display."
  (when (fboundp 'romacs--take-key-repeats)
    (let ((taken (romacs--take-key-repeats)))
      (when (and taken (or (> (nth 0 taken) 0) (> (nth 2 taken) 0)))
        (json-serialize (list :kind "repeat" :n (nth 0 taken) :bounds_ms romacs-usage-log--gap-bounds
                              :after_press_ms (nth 1 taken) :longest_hold_ms (nth 2 taken)))))))

(defun romacs-usage-log-flush ()
  "Append the queued lines and the counts since the last write to the log."
  (when romacs-usage-log-file
    (romacs-usage-log--charge t)
    (let ((counts nil))
      (maphash (lambda (mode table)
                 (maphash (lambda (command n)
                            (push (json-serialize (list :kind "command" :command (symbol-name command)
                                                        :mode (romacs-usage-log--name mode) :n n))
                                  counts))
                          table))
               romacs-usage-log--counts)
      (clrhash romacs-usage-log--counts)
      (when (fboundp 'romacs--layout-refusals)
        (dolist (refusal (romacs--layout-refusals))
          (push (json-serialize (list :kind "refusal" :class (romacs-usage-log--scrub (nth 0 refusal))
                                      :mode (nth 1 refusal) :n (nth 2 refusal)))
                counts)))
      (maphash (lambda (key slot)
                 (let ((space (string-search " " key)))
                   (push (json-serialize (list :kind "slow" :command (substring key 0 space)
                                               :mode (substring key (1+ space)) :n (aref slot 0)
                                               :ms (round (aref slot 1)) :redisplay_ms (round (aref slot 3))
                                               :max (round (aref slot 2))))
                         counts)))
               romacs-usage-log--slow)
      (clrhash romacs-usage-log--slow)
      (let ((repeat (romacs-usage-log--key-repeats)))
        (when repeat (push repeat counts)))
      (when (> (apply #'+ (append romacs-usage-log--gaps nil)) 0)
        (push (json-serialize (list :kind "gaps" :bounds_ms romacs-usage-log--gap-bounds
                                    :counts (copy-sequence romacs-usage-log--gaps)))
              counts)
        (fillarray romacs-usage-log--gaps 0))
      (let ((lines (nconc (nreverse romacs-usage-log--pending) (nreverse counts) (romacs-usage-log--collections))))
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
  ;; No redisplay comes now.
  (romacs-usage-log--charge)
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
  (setq romacs-usage-log--settling nil romacs-usage-log--redisplayed nil
        romacs-usage-log--idle-since nil)
  (fillarray romacs-usage-log--gaps 0)
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
