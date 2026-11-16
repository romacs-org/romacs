;;; romacs-progress-view.el --- A start's long steps, shown as they go  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A start's long steps (a restore's buffers, the agent sessions loading)
;; report through `romacs-startup-progress', which keeps each one's latest
;; state in `romacs-progress-state'.  The launch screen shows them while it
;; shows; past it, `romacs-startup-progress' calls
;; `romacs-progress-view-update', which shows the steps that have a state
;; in a card in the frame's corner, a line and a bar per step.  The card
;; goes once every step is done; a step that failed stays a while, and its
;; error is in *Messages* too.

;;; Code:

(require 'seq)
(require 'subr-x)

(declare-function romacs-popup-show "romacs-popup" (kind buffer &rest args))
(declare-function romacs-popup-hide "romacs-popup" (buffer))
(declare-function romacs-layer--root-frame "romacs-layer" ())

(defvar romacs-progress-state)

(defgroup romacs-progress-view nil
  "A start's long steps, shown as they go."
  :group 'frames)

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

(defun romacs-progress-view--line (step state width)
  "STEP at STATE as a line of at most WIDTH columns: a glyph, its words,
then its bar and counts, or its error.  The bar narrows before the words
do; a failure's words and error share what the line leaves, cut with an
ellipsis (the error is whole in *Messages*)."
  (let* ((status (plist-get state :status))
         (done (plist-get state :done)) (of (plist-get state :of))
         (glyph (pcase status
                  ('done (propertize "✓ " 'face 'romacs-progress-view-bar))
                  ('failed (propertize "✗ " 'face 'romacs-progress-view-failed))
                  (_ "  ")))
         (words (romacs-progress-view--words step state))
         ;; The glyph and the gap after the words.
         (room (max 0 (- width 4)))
         (cut (lambda (text columns) (truncate-string-to-width text (max 0 columns) nil nil "…"))))
    (if (eq status 'failed)
        (let* ((error (format "%s" (or (plist-get state :error) "failed")))
               (words-room (min (string-width words) (max (/ room 2) (- room (string-width error)))))
               (words (funcall cut words words-room)))
          (concat glyph words "  "
                  (propertize (funcall cut error (- room (string-width words))) 'face 'romacs-progress-view-failed)))
      (let* ((counts (and (natnump done) (natnump of) (format "  %d/%d" done of)))
             (bar (max 4 (min romacs-progress-view--bar (- room (length counts) 6))))
             (lit (if (and counts (> of 0))
                      (round (* bar (/ (float (min done of)) of)))
                    (if (eq status 'done) bar 0)))
             (tail (concat (propertize (make-string lit ?━) 'face 'romacs-progress-view-bar)
                           (propertize (make-string (- bar lit) ?─) 'face 'romacs-progress-view-rest)
                           (and counts (propertize counts 'face 'romacs-progress-view-rest)))))
        (concat glyph (funcall cut words (- room (string-width tail))) "  " tail)))))

(defun romacs-progress-view--steps ()
  "The steps of `romacs-progress-state' with a state, as (STEP . STATE),
in the order they first reported."
  (seq-filter (lambda (entry) (plist-get (cdr entry) :status)) (reverse (bound-and-true-p romacs-progress-state))))

(defun romacs-progress-view--card ()
  "Show every step with a state in the card, in the frame's bottom-right
corner; hide it a while after the last one ended.  The menus' library
loads here, not with this one, which a start may load early."
  (require 'romacs-popup)
  (let* ((root (romacs-layer--root-frame))
         (room (max 24 (min 80 (- (frame-width root) 8))))
         (steps (romacs-progress-view--steps))
         (lines (mapcar (lambda (entry) (romacs-progress-view--line (car entry) (cdr entry) room)) steps))
         (buffer (get-buffer-create romacs-progress-view--name))
         (columns (min room (+ 2 (apply #'max 20 (mapcar #'string-width lines))))))
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

(defvar romacs-progress-view--handoff nil
  "The timer that moves steps still going from the launch screen to the card.")

(defun romacs-progress-view--handoff ()
  "The launch screen gave way (timers run once the session's first frame
is up): the steps still going go on in the card."
  (setq romacs-progress-view--handoff nil)
  (when (seq-some (lambda (entry) (eq (plist-get (cdr entry) :status) 'running)) (romacs-progress-view--steps))
    (romacs-progress-view--card)))

(defvar romacs-progress-view--told nil
  "The failures said in *Messages* already, as (STEP . ERROR).")

(defun romacs-progress-view-update (&optional on-screen)
  "Show `romacs-progress-state' (`romacs-startup-progress' calls this when
a step with a state reports): in the card past the launch screen; a step
the screen shows (ON-SCREEN) that goes on moves to the card once the
session's first frame is up.  Each failure is said in *Messages* once."
  (dolist (entry (romacs-progress-view--steps))
    (let ((told (cons (car entry) (plist-get (cdr entry) :error))))
      (when (and (eq (plist-get (cdr entry) :status) 'failed) (not (member told romacs-progress-view--told)))
        (push told romacs-progress-view--told)
        (message "%s failed: %s" (romacs-progress-view--words (car entry) (cdr entry)) (or (cdr told) "")))))
  (cond (on-screen
         (unless romacs-progress-view--handoff
           (setq romacs-progress-view--handoff (run-at-time 0 nil #'romacs-progress-view--handoff))))
        ((and (display-graphic-p) (not noninteractive)
              ;; A card shows while a step goes on or failed; one already up
              ;; follows its steps to the end.
              (or (get-buffer-window romacs-progress-view--name 'visible)
                  (seq-some (lambda (entry) (memq (plist-get (cdr entry) :status) '(running failed)))
                            (romacs-progress-view--steps))))
         (romacs-progress-view--card))))

(provide 'romacs-progress-view)
;;; romacs-progress-view.el ends here
