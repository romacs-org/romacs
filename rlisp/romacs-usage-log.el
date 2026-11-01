;;; romacs-usage-log.el --- Which commands a session runs, in a local log  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

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
;;       S seconds after `before-init-time'.
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
  (push (vector (recursion-depth) (float-time) major-mode) romacs-usage-log--open))

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

(defun romacs-usage-log--command-error (data _context _signal)
  "Log the command loop's error DATA, before `command-error-function' reports it."
  (condition-case nil
      (romacs-usage-log--push :kind "error" :command (romacs-usage-log--name real-this-command)
                              :error (romacs-usage-log--error-text data))
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
  (romacs-usage-log--event "init-done"))

(defun romacs-usage-log--kill ()
  (romacs-usage-log--event "kill")
  (romacs-usage-log-flush))

(defun romacs-usage-log--enable (file)
  "Log this session's commands to FILE."
  (setq romacs-usage-log-file (expand-file-name file))
  (add-hook 'pre-command-hook #'romacs-usage-log--pre-command)
  (add-hook 'post-command-hook #'romacs-usage-log--post-command)
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
  (remove-hook 'after-init-hook #'romacs-usage-log--init-done)
  (remove-hook 'kill-emacs-hook #'romacs-usage-log--kill)
  (remove-function command-error-function #'romacs-usage-log--command-error)
  (when romacs-usage-log--timer (cancel-timer romacs-usage-log--timer))
  (setq romacs-usage-log--timer nil))

(defun romacs-usage-log-start ()
  "Log this session's commands to the file ROMACS_USAGE_LOG names, if it
names one and the session is interactive."
  (let ((file (getenv "ROMACS_USAGE_LOG")))
    (when (and file (not (string-empty-p file)) (not noninteractive))
      (romacs-usage-log--enable file))))

(provide 'romacs-usage-log)
;;; romacs-usage-log.el ends here
