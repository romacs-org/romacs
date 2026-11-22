;;; romacs-agent-log-activity.el --- Active subagents beside the composer -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The live part of the chat is one EWOC, just above its divider.
;; Worker state belongs to the session.  This view only holds its rendered
;; lines and their bounds, so they never become part of a replayed transcript.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'seq)
(require 'ewoc)
(require 'romacs-working)

(defface romacs-agent-log-subagent '((t :inherit font-lock-keyword-face :weight bold))
  "The name and animation of a running subagent in chat.")

(defvar-local romacs-agent-log-activity--view nil
  "The live chat's (EWOC START . END), or nil.")

(defvar-local romacs-agent-log-activity--threads nil
  "The working threads in the live activity, from id to their marker.")

(defconst romacs-agent-log-activity--cols 4
  "How many cells a subagent's animation fills.")

(defun romacs-agent-log-activity--running-p (row)
  "Whether session worker ROW belongs in the live subagent view."
  (and (plist-get row :subagent) (equal (plist-get row :state) "running")))

(defun romacs-agent-log-activity--before-p (a b)
  "Whether A started before B, using their tool ids to break a tie."
  (let ((x (or (plist-get a :started) 0))
        (y (or (plist-get b :started) 0)))
    (if (= x y) (string< (plist-get a :tool) (plist-get b :tool)) (< x y))))

(defun romacs-agent-log-activity--line (row width)
  "Draw worker ROW on one line, within WIDTH columns."
  (let* ((title (or (plist-get row :title) "Subagent"))
         (activity (or (plist-get row :activity) "Working"))
         ;; The display overlays working cells at the start of their line.
         (prefix (when (> width (+ romacs-agent-log-activity--cols 8))
                   (concat (propertize (romacs-working-still romacs-agent-log-activity--cols 'running
                                                             'romacs-agent-log-subagent)
                                       'romacs-agent-worker (plist-get row :tool)) "  ")))
         (room (max 1 (- width (length prefix) 1)))
         (name (romacs-agent-log-activity--clip title (if (> room 14) (min 40 (/ room 2)) room)))
         (action (- room (string-width name) 5)))
    (concat (propertize (concat prefix (propertize name 'face 'romacs-agent-log-subagent)
                                 (when (> action 2)
                                   (propertize (concat "  ·  " (romacs-agent-log-activity--clip activity action))
                                               'face 'romacs-agent-log-note)))
                        'help-echo (concat title " · " activity))
            "\n")))

(defun romacs-agent-log-activity--clip (text width)
  "A single line of TEXT that fits WIDTH."
  (truncate-string-to-width (replace-regexp-in-string "[[:space:]]+" " " text)
                            width nil nil (truncate-string-ellipsis)))

(defun romacs-agent-log-activity-lines (rows width)
  "Render the running subagents in worker ROWS, within WIDTH."
  (if-let* ((active (seq-filter #'romacs-agent-log-activity--running-p rows)))
      (concat (propertize (truncate-string-to-width (format "Subagents (%d)" (length active)) (max 1 (1- width)))
                          'face 'romacs-agent-log-note)
              "\n"
              (mapconcat (lambda (row) (romacs-agent-log-activity--line row width))
                         (sort active #'romacs-agent-log-activity--before-p) ""))
    ""))

(defun romacs-agent-log-activity-bounds ()
  "The active view's (START . END) markers, or nil."
  (cdr romacs-agent-log-activity--view))

(defun romacs-agent-log-activity--animate (&optional start end)
  "Attach the working animations to the rows between START and END."
  (let ((old romacs-agent-log-activity--threads) (next nil) (at start))
    (while (and at (setq at (text-property-not-all at end 'romacs-agent-worker nil)))
      (let* ((id (list (current-buffer) (get-text-property at 'romacs-agent-worker)))
             (entry (assoc id old))
             (marker (or (cdr entry) (copy-marker at))))
        (if entry (set-marker marker at)
          (romacs-working-show id (current-buffer) marker romacs-agent-log-activity--cols
                               'running 'romacs-agent-log-subagent))
        (push (cons id marker) next)
        (setq at (next-single-property-change at 'romacs-agent-worker nil end))))
    (dolist (entry old)
      (unless (assoc (car entry) next)
        (romacs-working-stop (car entry))
        (set-marker (cdr entry) nil)))
    (setq romacs-agent-log-activity--threads next)))

(defun romacs-agent-log-activity-set (text &optional at)
  "Show sealed TEXT in the live view.  Create it at AT, or clear it if empty."
  (save-restriction
    (widen)
    (with-silent-modifications
      (save-excursion
        (pcase romacs-agent-log-activity--view
          (`(,ewoc ,start . ,end)
           (let ((node (ewoc-nth ewoc 0)))
             (cond ((string-empty-p text)
                    (romacs-agent-log-activity--animate)
                    (ewoc-delete ewoc node)
                    (delete-region (1- start) start)
                    (set-marker start nil)
                    (set-marker end nil)
                    (setq romacs-agent-log-activity--view nil))
                   ((not (equal-including-properties text (ewoc-data node)))
                    (ewoc-set-data node text)
                    (ewoc-invalidate ewoc node)
                    (romacs-agent-log-activity--animate start end)))))
          (_ (unless (string-empty-p text)
               (goto-char at)
               ;; The transcript grows before this newline.  Its insertions
               ;; cannot share the EWOC's first marker.
               (insert (substring text -1))
               (let ((start (point-marker))
                     (end (copy-marker (point) t))
                     (ewoc (ewoc-create #'insert nil nil t)))
                 (ewoc-enter-last ewoc text)
                 (setq romacs-agent-log-activity--view (cons ewoc (cons start end)))
                 (romacs-agent-log-activity--animate start end)))))))))

(provide 'romacs-agent-log-activity)
;;; romacs-agent-log-activity.el ends here
