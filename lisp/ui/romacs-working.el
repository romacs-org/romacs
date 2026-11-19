;;; romacs-working.el --- A thread that says how work goes  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The working thread: a row of cells in a buffer's text that the render
;; thread animates by itself (romacs-animate.el), so it moves while the
;; evaluator is busy, and so tells "alive, working" from "frozen".  Its
;; motion says how a piece of work goes:
;;
;;   idle     slack and still
;;   running  a wave travels along it, a bead riding it
;;   waiting  taut, pulsing: the work waits on its user
;;   failed   cut, still
;;
;; Any module shows one over text of its own (a long command, a mail
;; send, a build, an agent's turn):
;;
;;   (romacs-working-show ID BUFFER START COLS &optional WORK)
;;   (romacs-working-set ID WORK)
;;   (romacs-working-stop ID)
;;
;; START is where a line of BUFFER begins whose first COLS characters the
;; thread is drawn over, a position or a marker of the caller's that it
;; follows; `romacs-working-still' gives text for them, the frame shown
;; where nothing animates.  Each state's colour is a face's.

;;; Code:

(require 'romacs-animate)

(defface romacs-working-idle '((t :inherit shadow)) "The working thread, idle.")
(defface romacs-working-running '((t :inherit font-lock-function-name-face)) "The working thread, running.")
(defface romacs-working-waiting '((t :inherit warning)) "The working thread, waiting on its user.")
(defface romacs-working-failed '((t :inherit error)) "The working thread, failed.")

(defconst romacs-working-fps 30 "Frames a second of a moving thread.")

(defvar romacs-working--threads (make-hash-table :test #'equal)
  "Each thread shown, by id: a plist of :start (a marker), :own (whether
it made the marker), :cols, :work and :epoch.")

(defun romacs-working--face (work)
  (intern (format "romacs-working-%s" work)))

(defun romacs-working-still (cols work)
  "COLS characters of the thread for WORK at rest, in its face."
  (propertize (if (eq work 'failed)
                  (let ((gap (max 1 (/ cols 6))))
                    (concat (make-string (- (/ cols 2) gap) ?⠤) (make-string gap ?\s)
                            (make-string (- cols (- (/ cols 2) gap) gap) ?⠤)))
                (make-string cols (if (eq work 'waiting) ?⠒ ?⠤)))
              'face (romacs-working--face work)))

(defun romacs-working--element (id)
  "What the display draws of thread ID: its cells, its state and colour."
  (when-let* ((thread (gethash id romacs-working--threads))
              (start (marker-position (plist-get thread :start))))
    (list :start start :cols (plist-get thread :cols) :rows 1
          :kind (list 'working (plist-get thread :work) (plist-get thread :epoch)
                      (romacs-working--face (plist-get thread :work))))))

(defun romacs-working--motion (id)
  "Let thread ID move only while its work runs or waits."
  (if (memq (plist-get (gethash id romacs-working--threads) :work) '(running waiting))
      (romacs-animate-resume id)
    (romacs-animate-pause id)))

(defun romacs-working-show (id buffer start cols &optional work)
  "Show thread ID over the COLS characters from START, a line's start in
BUFFER (a marker, followed as it moves), its work WORK (`idle' by
default).  A thread ID shown already moves there."
  (romacs-working-stop id)
  (puthash id (list :start (if (markerp start) start (with-current-buffer buffer (copy-marker start)))
                    :own (not (markerp start)) :cols cols :work (or work 'idle) :epoch 0)
           romacs-working--threads)
  (romacs-animate-start id buffer romacs-working-fps #'ignore
                        (lambda (_window) (romacs-working--element id)))
  (romacs-working--motion id)
  id)

(defun romacs-working-set (id work)
  "Thread ID's work goes as WORK now: `idle', `running', `waiting' or `failed'."
  (when-let* ((thread (gethash id romacs-working--threads))
              ((not (eq (plist-get thread :work) work))))
    (puthash id (plist-put thread :work work) romacs-working--threads)
    (romacs-working--motion id)
    (romacs-animate-redraw id)))

(defun romacs-working-stop (id)
  "Show thread ID no more."
  (when-let* ((thread (gethash id romacs-working--threads)))
    (when (plist-get thread :own) (set-marker (plist-get thread :start) nil))
    (remhash id romacs-working--threads)
    (romacs-animate-stop id)))

(provide 'romacs-working)
;;; romacs-working.el ends here
