;;; romacs-test-notes.el --- A notes-only host for agent tests -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; A session with no supervisor has its notes written by a notes-only
;; host, a process started for it (`romacs-agent--notes-host').  Tests of
;; what a note does put this one in its place: it writes each held note
;; as the next record and echoes it, as the host does, and nothing else.
;; The host itself is tested against the real binary.

;;; Code:

(require 'romacs-agent)

(defun romacs-test-notes-host (session)
  "Write SESSION's held notes as its next records, and echo each."
  (dolist (note (gethash (romacs-agent-id session) romacs-agent--held-notes))
    (let ((record (romacs-mcp--object "seq" (1+ (romacs-agent-seq session)) "t" (float-time) "note" note)))
      (if (romacs-agent-events-p session) (romacs-agent-events-append session record)
        (let ((coding-system-for-write 'utf-8-unix))
          (write-region (concat (romacs-agent--json record) "\n") nil
                        (expand-file-name "journal.jsonl" (romacs-agent-dir session)) t 'silent)))
      (romacs-agent--held-echoed session record)
      (romacs-agent--record session record))))

(setq romacs-agent--notes-host #'romacs-test-notes-host)

(provide 'romacs-test-notes)
;;; romacs-test-notes.el ends here
