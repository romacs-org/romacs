;;; romacs-agent-transcript.el --- Read a harness transcript -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Read bounded JSONL records and normalize shared transcript fields.

;;; Code:

(require 'subr-x)
(declare-function romacs-agent--task-line "romacs-agent" (text))

(defun romacs-agent-transcript-read (file beg end keep)
  "The JSON objects of FILE's complete lines between bytes BEG and END, in
order, that KEEP (a function of a parsed object) accepts.  Each line is
parsed whole, however its writer spaced or ordered it.  A line BEG or
END cuts is left out; the file's last line counts whole without its
newline when END is the file's end."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    ;; From the byte before BEG: a line starting at BEG is whole.
    (insert-file-contents-literally file nil (max 0 (1- beg)) end)
    (goto-char (point-min))
    ;; Cut at the start: a line's tail, or the newline before BEG.
    (when (> beg 0) (forward-line 1))
    (let ((objects nil)
          (whole-end (>= end (or (file-attribute-size (file-attributes file)) end))))
      (while (< (point) (point-max))
        (let* ((line-start (point))
               (newline (search-forward "\n" nil t))
               (line-end (if newline (1- newline) (point-max))))
          (when (or newline whole-end)
            (let ((object (ignore-errors
                            (json-parse-string (decode-coding-string (buffer-substring-no-properties line-start line-end)
                                                                     'utf-8)
                                               :object-type 'hash-table :null-object :null :false-object :false))))
              (when (and (hash-table-p object) (funcall keep object)) (push object objects))))
          (goto-char (if newline newline (point-max)))))
      (nreverse objects))))

(defun romacs-agent-transcript-title (text)
  "TEXT's first line, as a title (`romacs-agent--task-line'); nil for
none, or for a harness's own injected context (a <tag>, AGENTS.md's
instructions)."
  (let ((text (and (stringp text) (string-trim text))))
    (unless (or (null text) (string-empty-p text) (string-prefix-p "<" text)
                (string-prefix-p "# AGENTS.md instructions" text))
      (truncate-string-to-width (romacs-agent--task-line text) 60 nil nil t))))

(defun romacs-agent-transcript--time (stamp)
  (and (stringp stamp) (ignore-errors (float-time (date-to-time stamp)))))

(defun romacs-agent-transcript-duration (start end)
  "Seconds from timestamp START to END, or nil when either is unknown."
  (let ((start (romacs-agent-transcript--time start))
        (end (romacs-agent-transcript--time end)))
    (and start end (max 0 (- end start)))))

(provide 'romacs-agent-transcript)
;;; romacs-agent-transcript.el ends here
