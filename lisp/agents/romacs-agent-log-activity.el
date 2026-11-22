;;; romacs-agent-log-activity.el --- Active subagents beside the composer -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The live part of the chat is one EWOC, between its divider and prompt.
;; Worker state belongs to the session.  This view only holds its rendered
;; lines and their bounds, so they never become part of a replayed transcript.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'seq)
(require 'ewoc)
(require 'romacs-agent-glyphs)

(defvar-local romacs-agent-log-activity--view nil
  "The live chat's (EWOC START . END), or nil.")

(defun romacs-agent-log-activity--running-p (row)
  "Whether session worker ROW belongs in the live subagent view."
  (and (plist-get row :subagent) (equal (plist-get row :state) "running")))

(defun romacs-agent-log-activity--before-p (a b)
  "Whether A started before B, using their tool ids to break a tie."
  (let ((x (or (plist-get a :started) 0))
        (y (or (plist-get b :started) 0)))
    (if (= x y) (string< (plist-get a :tool) (plist-get b :tool)) (< x y))))

(defun romacs-agent-log-activity-lines (rows width)
  "Render the running subagents in worker ROWS, within WIDTH."
  (mapconcat
   (lambda (row)
     (let* ((title (or (plist-get row :title) "Subagent"))
            (activity (plist-get row :activity))
            (text (string-join (delq nil (list title activity)) " · ")))
       (concat (propertize
                (truncate-string-to-width
                 (concat "  " (romacs-agent-glyph 'working) " "
                         (replace-regexp-in-string "[[:space:]]+" " " text))
                 (max 1 (1- width)) nil nil (truncate-string-ellipsis))
                'face 'romacs-agent-log-note 'help-echo text)
               "\n")))
   (sort (seq-filter #'romacs-agent-log-activity--running-p rows)
         #'romacs-agent-log-activity--before-p)
   ""))

(defun romacs-agent-log-activity-bounds ()
  "The active view's (START . END) markers, or nil."
  (cdr romacs-agent-log-activity--view))

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
                    (ewoc-delete ewoc node)
                    (set-marker start nil)
                    (set-marker end nil)
                    (setq romacs-agent-log-activity--view nil))
                   ((not (equal-including-properties text (ewoc-data node)))
                    (ewoc-set-data node text)
                    (ewoc-invalidate ewoc node)))))
          (_ (unless (string-empty-p text)
               (goto-char at)
               (let ((start (point-marker))
                     (end (copy-marker (point) t))
                     (ewoc (ewoc-create #'insert nil nil t)))
                 (ewoc-enter-last ewoc text)
                 (setq romacs-agent-log-activity--view (cons ewoc (cons start end)))))))))))

(provide 'romacs-agent-log-activity)
;;; romacs-agent-log-activity.el ends here
