;;; romacs-agent-log-turns.el --- An agent chat's turns in its transcript  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Where a chat's turns are (romacs-agent-log.el): each prompt names the
;; turn it starts by its request, so the turn at point is known (a diff
;; or a rollback of it), and a rollback dims the turns it took back, on
;; whichever page draws them: one rollback at a time, searching back from
;; its note, and a prompt not drawn yet kept until earlier turns come
;; above.

;;; Code:

(require 'romacs-agent-log)

(defvar-local romacs-agent-log-turns--rollbacks nil
  "Rollbacks whose prompt is among the turns not drawn: (TO . NOTE), TO
the prompt's request, NOTE a marker at the rollback's note.")

(defun romacs-agent-log-turns-rolled-back (event)
  "Draw rollback EVENT: a note of how many turns, whether their files came
back, and to before which prompt; the turns it took back are dimmed
\(`romacs-agent-log-turns--dim-rollback')."
  (let ((turns (or (plist-get event :turns) 0)))
    (romacs-agent-log--line event 'note
                            (propertize (format "· ↶ rolled back %d turn%s%s to before: %s" turns (if (= turns 1) "" "s")
                                                (if (plist-get event :files) " and their files" "")
                                                (truncate-string-to-width (car (split-string (or (plist-get event :prompt) "") "\n"))
                                                                          60 nil nil t))
                                        'romacs-agent-log-rolled-back (plist-get event :to))
                            'romacs-agent-log-note)
    ;; The note just drawn, at the transcript's end.
    (let ((pos romacs-agent-log--end))
      (while (and (> pos (point-min)) (not (get-text-property (1- pos) 'romacs-agent-log-rolled-back)))
        (setq pos (1- pos)))
      (when (> pos (point-min))
        (romacs-agent-log-turns--dim-rollback (plist-get event :to)
                                        (previous-single-property-change pos 'romacs-agent-log-rolled-back nil (point-min)))))))

(defun romacs-agent-log-turns--dim (start end)
  "Dim START to END, what is not dimmed yet."
  (let ((pos start))
    (while (< pos end)
      (let ((next (next-single-property-change pos 'romacs-agent-log-dimmed nil end)))
        (unless (get-text-property pos 'romacs-agent-log-dimmed)
          (add-face-text-property pos next 'romacs-agent-log-note)
          (put-text-property pos next 'romacs-agent-log-dimmed t))
        (setq pos next)))))

(defun romacs-agent-log-turns--prompt-before (to note &optional bound)
  "Where the prompt of turn TO starts, searched back from NOTE to BOUND
\(the transcript's start by default), a run of text at a time; nil when
not there."
  (let ((pos note) (bound (or bound (point-min))) (found nil))
    (while (and (not found) (> pos bound))
      (setq pos (or (previous-single-property-change pos 'romacs-agent-log-turn nil bound) bound))
      (when (equal (get-text-property pos 'romacs-agent-log-turn) to) (setq found pos)))
    found))

(defun romacs-agent-log-turns--dim-rollback (to note)
  "Dim the turns the rollback to before TO's prompt took back, up to its
NOTE: from that prompt, searched back from the note; when it is among
the turns not drawn, from the earlier turns' row, kept to finish as
earlier turns come above (`romacs-agent-log-turns--dim-above').  They stay
readable, as the records keep them."
  (romacs-agent-log--writing
    (if-let* ((start (romacs-agent-log-turns--prompt-before to note)))
        (romacs-agent-log-turns--dim start note)
      (when-let* (((get-text-property (point-min) 'romacs-agent-log-earlier))
                  (row-end (next-single-property-change (point-min) 'romacs-agent-log-earlier nil note)))
        (romacs-agent-log-turns--dim row-end note))
      (push (cons to (copy-marker note)) romacs-agent-log-turns--rollbacks))))

(defun romacs-agent-log-turns--dim-above (end)
  "Earlier turns were put above, up to END: the rollbacks waiting for a
prompt among them are dimmed from it, and those they draw whose prompt is
still not drawn are dimmed from the new earlier turns' row, and wait."
  (romacs-agent-log--writing
    (setq romacs-agent-log-turns--rollbacks
          (seq-remove (lambda (pending)
                        (when-let* ((start (romacs-agent-log-turns--prompt-before (car pending) end)))
                          (romacs-agent-log-turns--dim start (cdr pending))
                          (set-marker (cdr pending) nil)
                          t))
                      romacs-agent-log-turns--rollbacks))
    (let ((pos (point-min)))
      (while (setq pos (text-property-not-all pos end 'romacs-agent-log-rolled-back nil))
        (let ((to (get-text-property pos 'romacs-agent-log-rolled-back)))
          (unless (romacs-agent-log-turns--prompt-before to pos)
            (romacs-agent-log-turns--dim-rollback to pos))
          (setq pos (next-single-property-change pos 'romacs-agent-log-rolled-back nil end)))))))

(defun romacs-agent-log-turns-here ()
  "The request id of the turn at point: its prompt's, at or before point;
nil before the first."
  (save-excursion
    (let ((pos (min (point) (max (point-min) (1- romacs-agent-log--end)))))
      (while (and (> pos (point-min)) (not (get-text-property pos 'romacs-agent-log-turn)))
        (setq pos (or (previous-single-property-change pos 'romacs-agent-log-turn) (point-min))))
      (get-text-property pos 'romacs-agent-log-turn))))

(defun romacs-agent-log-turns-text (request)
  "The prompt of the turn REQUEST starts, as the chat shows it, or nil
when the chat does not show it."
  (when-let* ((at (text-property-any (point-min) romacs-agent-log--end 'romacs-agent-log-turn request)))
    (save-excursion
      (goto-char at)
      (skip-chars-forward "\n")
      (string-trim (buffer-substring-no-properties (point) (line-end-position))))))

(provide 'romacs-agent-log-turns)
;;; romacs-agent-log-turns.el ends here
