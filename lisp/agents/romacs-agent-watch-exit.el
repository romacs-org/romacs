;;; romacs-agent-watch-exit.el --- Exit records for watched units -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The launcher and watcher share a file per unit under
;; $XDG_RUNTIME_DIR/unit-exit.  ExecStopPost publishes the record by rename,
;; before the unit is collected.

;;; Code:

(require 'subr-x)
(require 'rx)

(defun romacs-agent-watch--exit-properties (unit)
  "Return systemd-run property words recording the exit of full unit name UNIT.
None when mv cannot be found: the record is best effort, never a reason
not to launch, and a unit without one is said to have none."
  (let ((case-fold-search nil))
    (unless (string-match-p "\\`[A-Za-z0-9:_.-]+\\'" unit)
      (user-error "Invalid unit name for an exit record: %s" unit)))
  (when-let* ((mv (executable-find "mv")))
    (let ((print-escape-newlines t))
      (list "--property=RuntimeDirectory=unit-exit"
            "--property=RuntimeDirectoryPreserve=yes"
            (format (concat "--property=ExecStopPost=-:/bin/sh -c 'd=\"$RUNTIME_DIRECTORY\"; "
                            "echo \"$SERVICE_RESULT $EXIT_CODE $EXIT_STATUS $INVOCATION_ID\" > \"$d/.$$.tmp\""
                            " && exec \"$1\" -f \"$d/.$$.tmp\" \"$d/%s\"' sh %s")
                    unit (prin1-to-string (expand-file-name mv)))))))

(defun romacs-agent-watch--parse-exit-record (line)
  "Parse LINE as (SERVICE-RESULT EXIT-CODE EXIT-STATUS INVOCATION-ID), or nil."
  (let ((case-fold-search nil))
    (save-match-data
      (when (string-match
             (rx string-start (group (+ (any "a-z-"))) " "
                 (group (or "exited" "killed" "dumped")) " "
                 (group (+ (any "A-Z0-9")) (? (any "+-") (+ digit))) " "
                 (group (= 32 (any "0-9a-fA-F"))) (? "\n") string-end)
             line)
        (let ((code (match-string 2 line))
              (status (match-string 3 line)))
          ;; An exit is a status 0-255; a death is a signal's name,
          ;; a real-time one with its offset (RTMIN+1).
          (when (if (equal code "exited")
                    (and (string-match-p (rx string-start (** 1 3 digit) string-end) status)
                         (<= (string-to-number status) 255))
                  (string-match-p (rx string-start alpha) status))
            (list (match-string 1 line) code status (match-string 4 line))))))))

(defun romacs-agent-watch--exit-record (unit invocation)
  "Read full unit name UNIT's exit record for INVOCATION, or return nil."
  (when-let* (invocation
              (runtime (getenv "XDG_RUNTIME_DIR"))
              ((not (string-empty-p runtime)))
              ((equal unit (file-name-nondirectory unit)))
              (file (expand-file-name (concat "unit-exit/" unit) runtime)))
    (condition-case nil
        (when (file-regular-p file)
          (with-temp-buffer
            (insert-file-contents file)
            (let ((record (romacs-agent-watch--parse-exit-record (buffer-string))))
              (when (equal (nth 3 record) invocation) record))))
      (file-error nil))))

(provide 'romacs-agent-watch-exit)
;;; romacs-agent-watch-exit.el ends here
