;;; romacs-progress-view.el --- A start's long steps, shown as they go  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; What `romacs-progress-functions' reports of a start's long steps (a
;; restore's buffers, the agent sessions loading), shown where the user
;; looks: on the launch screen while it shows (`romacs-startup-progress'),
;; else in a card in the frame's corner, a line and a bar per step.  The
;; card goes once every step is done; a step that failed stays a while, and
;; its error is in *Messages* too.

;;; Code:

(require 'seq)
(require 'subr-x)
(require 'romacs-popup)

(declare-function romacs-startup-progress "animate.rs" (stage &optional label done of failed))
(defvar romacs-progress-state)

(defgroup romacs-progress-view nil
  "A start's long steps, shown as they go."
  :group 'romacs-popup)

(defface romacs-progress-view-bar '((t :inherit success))
  "The part of a step's bar done.")

(defface romacs-progress-view-rest '((t :inherit shadow))
  "The part of a step's bar to go, and its counts.")

(defface romacs-progress-view-failed '((t :inherit error))
  "A step that failed.")

(defcustom romacs-progress-view-linger 1.5
  "Seconds the card stays once every step is done."
  :type 'number)

(defcustom romacs-progress-view-failed-linger 10
  "Seconds the card stays once every step ended, one of them failed."
  :type 'number)

(defcustom romacs-progress-view-steps
  '((restore . "Restoring buffers") (agents . "Loading agent sessions") (init . "Loading your init"))
  "What each step is called; a step not here is called by its name."
  :type '(alist :key-type symbol :value-type string))

(defconst romacs-progress-view--name " *romacs-progress*")

(defconst romacs-progress-view--bar 16
  "A step's bar, in columns.")

(defvar romacs-progress-view--timer nil
  "The timer that hides the card once every step ended.")

(defun romacs-progress-view--words (step state)
  "What STEP at STATE is doing, in words."
  (let ((words (or (alist-get step romacs-progress-view-steps) (capitalize (symbol-name step))))
        (name (plist-get state :name)))
    (if (and (stringp name) (not (string-empty-p name))) (format "%s: %s" words name) words)))

(defun romacs-progress-view--line (step state)
  "STEP at STATE as a line: a glyph, its words, its bar and counts."
  (let* ((status (plist-get state :status))
         (done (plist-get state :done)) (of (plist-get state :of))
         (lit (if (and (natnump done) (natnump of) (> of 0))
                  (round (* romacs-progress-view--bar (/ (float (min done of)) of)))
                (if (eq status 'done) romacs-progress-view--bar 0))))
    (concat (pcase status
              ('done (propertize "✓ " 'face 'romacs-progress-view-bar))
              ('failed (propertize "✗ " 'face 'romacs-progress-view-failed))
              (_ "  "))
            (romacs-progress-view--words step state)
            "  "
            (if (eq status 'failed)
                (propertize (format "%s" (or (plist-get state :error) "failed")) 'face 'romacs-progress-view-failed)
              (concat (propertize (make-string lit ?━) 'face 'romacs-progress-view-bar)
                      (propertize (make-string (- romacs-progress-view--bar lit) ?─) 'face 'romacs-progress-view-rest)
                      (and (natnump done) (natnump of)
                           (propertize (format "  %d/%d" done of) 'face 'romacs-progress-view-rest)))))))

(defun romacs-progress-view--card ()
  "Show every step of `romacs-progress-state' in the card, in the frame's
bottom-right corner; hide it a while after the last one ended."
  (let* ((steps (reverse romacs-progress-state))
         (lines (mapcar (lambda (entry) (romacs-progress-view--line (car entry) (cdr entry))) steps))
         (buffer (get-buffer-create romacs-progress-view--name))
         (columns (+ 2 (apply #'max 20 (mapcar #'string-width lines))))
         (root (romacs-layer--root-frame)))
    (with-current-buffer buffer
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert (string-join lines "\n")))
      (setq-local mode-line-format nil cursor-type nil truncate-lines t))
    (romacs-popup-show 'card buffer :columns columns :lines (length lines)
                       :anchor (cons (max 0 (- (frame-text-width root) (* (frame-char-width root) (+ columns 4))))
                                     (max 0 (- (frame-text-height root) (* (frame-char-height root) (+ (length lines) 4))))))
    (when romacs-progress-view--timer (cancel-timer romacs-progress-view--timer))
    (setq romacs-progress-view--timer
          (and (not (seq-some (lambda (entry) (eq (plist-get (cdr entry) :status) 'running)) steps))
               (run-at-time (if (seq-some (lambda (entry) (eq (plist-get (cdr entry) :status) 'failed)) steps)
                                romacs-progress-view-failed-linger
                              romacs-progress-view-linger)
                            nil #'romacs-progress-view--hide)))))

(defun romacs-progress-view--hide ()
  "Hide the card."
  (setq romacs-progress-view--timer nil)
  (when (get-buffer romacs-progress-view--name) (romacs-popup-hide romacs-progress-view--name)))

(defun romacs-progress-view-update (step state)
  "Show STEP at STATE (`romacs-progress-functions'): on the launch screen
while it shows, else in the frame's card; a failure in *Messages* too."
  (let ((failed (eq (plist-get state :status) 'failed)))
    (when failed
      (message "%s failed: %s" (romacs-progress-view--words step state) (or (plist-get state :error) "")))
    (unless (and (fboundp 'romacs-startup-progress)
                 (romacs-startup-progress step (romacs-progress-view--words step state)
                                          (plist-get state :done) (plist-get state :of) failed))
      (when (and (display-graphic-p) (not noninteractive))
        (romacs-progress-view--card)))))

(provide 'romacs-progress-view)
;;; romacs-progress-view.el ends here
