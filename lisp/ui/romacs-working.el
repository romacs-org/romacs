;;; romacs-working.el --- A thread that says how work goes  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The working thread: a row of cells in a buffer's text that the render
;; thread animates by itself (romacs-animate.el), so it moves while the
;; evaluator is busy, and tells "alive, working" from "frozen".  It says
;; how a piece of work goes:
;;
;;   idle     slack and still: a sparse, sagging line
;;   running  a wave travels along it; a bead rides it, one step for
;;            each piece of progress the work reports
;;   waiting  taut and solid, pulsing: the work waits on its user
;;   failed   cut, still
;;
;; Any module shows one over COLS characters of its own text (a long
;; command, a mail send, a build, an agent's turn):
;;
;;   (romacs-working-show ID BUFFER START COLS &optional WORK)
;;   (romacs-working-set ID WORK)
;;   (romacs-working-advance ID &optional N)  ; N pieces of progress
;;   (romacs-working-stop ID)
;;
;; START is where a line of BUFFER begins: a position, or a marker of the
;; caller's that the thread follows as text before it changes.  The
;; caller puts `romacs-working-still' there, COLS braille characters; the
;; thread then owns their look: it rewrites its still frame in place (no
;; marker moves, nothing is recorded to undo), and the display draws its
;; motion over them.  Each state's colour is a
;; face's.  At rest (idle, failed) the display draws nothing for it.
;;
;; Running and waiting are colours of the theme's own that cannot read as
;; an error: running its accent, else its blue or cyan, else what links
;; are drawn in; waiting its warning colour, else its yellow or orange;
;; each the first far from what `error' is drawn in and from the
;; background (`romacs-working--derive', on every theme change).  So
;; whatever the theme's syntax faces are, neither looks failed, and each
;; shows on a light theme as on a dark one.

;;; Code:

(require 'cl-lib)
(require 'romacs-animate)

(defface romacs-working-idle '((t :inherit shadow)) "The working thread, idle.")
(defface romacs-working-running '((t :inherit link :underline nil))
  "Work that runs: the working thread, and every view's running mark.
Its colour is derived from the theme (`romacs-working--derive').")
(defface romacs-working-waiting '((t :inherit warning))
  "Work that waits on its user: the working thread, and every view's
mark of it.  Its colour is derived from the theme (`romacs-working--derive').")
(defface romacs-working-failed '((t :inherit error)) "The working thread, failed.")

(defconst romacs-working-fps 30 "Frames a second of a moving thread.")

(declare-function romacs-theme-color "romacs-theme" (slot &optional theme))
(declare-function color-name-to-rgb "color" (color &optional frame))

(defun romacs-working--rgb (colour)
  "COLOUR as (R G B) in [0, 1]: a #rrggbb's own channels, else as the
frame resolves its name; nil when it cannot."
  (if (and (stringp colour) (string-match "\\`#\\([[:xdigit:]]\\{2\\}\\)\\([[:xdigit:]]\\{2\\}\\)\\([[:xdigit:]]\\{2\\}\\)\\'" colour))
      (mapcar (lambda (i) (/ (string-to-number (match-string i colour) 16) 255.0)) '(1 2 3))
    (ignore-errors (color-name-to-rgb colour))))

(defun romacs-working--far-p (colour others)
  "Whether COLOUR is far from each of OTHERS, colours: their RGB distance
over a quarter of the widest (a red and an amber are far)."
  (when-let* ((rgb (romacs-working--rgb colour)))
    (cl-every (lambda (other)
                (let ((o (and other (romacs-working--rgb other))))
                  (or (null o) (> (sqrt (apply #'+ (cl-mapcar (lambda (a b) (expt (- a b) 2)) rgb o))) 0.25))))
              others)))

(defun romacs-working--colour (candidates)
  "The first of CANDIDATES, colours, far from what `error' is drawn in and
from the background; else the theme's foreground."
  (let ((avoid (list (face-foreground 'error nil t) (face-background 'default nil t))))
    (or (cl-find-if (lambda (colour) (and (stringp colour) (romacs-working--far-p colour avoid))) candidates)
        (face-foreground 'default nil t))))

(defun romacs-working--palette (slot)
  (and (fboundp 'romacs-theme-color) (romacs-theme-color slot)))

(defun romacs-working--running-colour ()
  "The theme's colour for running work: its accent, blue, cyan or link
colour, the first far from its error colour and its background."
  (romacs-working--colour
   (list (romacs-working--palette 'accent) (romacs-working--palette 'blue) (romacs-working--palette 'cyan)
         (face-foreground 'link nil t) (face-foreground 'minibuffer-prompt nil t))))

(defun romacs-working--waiting-colour ()
  "The theme's colour for work waiting on its user: its warning, yellow or
orange colour, the first far from its error colour and its background."
  (romacs-working--colour
   (list (face-foreground 'warning nil t) (romacs-working--palette 'yellow) (romacs-working--palette 'orange)
         (romacs-working--palette 'bright-yellow))))

(defun romacs-working--derive (&rest _)
  "Make the running and waiting faces the theme's colours for them, as
their default specs: a theme's or the user's own spec still wins."
  (pcase-dolist (`(,face . ,colour) `((romacs-working-running . ,(romacs-working--running-colour))
                                     (romacs-working-waiting . ,(romacs-working--waiting-colour))))
    (face-spec-set face (if colour `((t :foreground ,colour)) '((t :inherit default))) 'face-defface-spec)))

(add-hook 'enable-theme-functions #'romacs-working--derive)
(add-hook 'disable-theme-functions #'romacs-working--derive)
(romacs-working--derive)

(defvar romacs-working--threads (make-hash-table :test #'equal)
  "Each thread shown, by id: a plist of :buffer, :start (a marker), :own
\(whether the thread made the marker), :cols, :work, :beats and
:animated (whether the display draws its motion).")

(defun romacs-working--face (work)
  (intern (format "romacs-working-%s" work)))

(defun romacs-working--moving-p (work)
  (memq work '(running waiting)))

(defun romacs-working-still (cols work)
  "COLS characters of the thread for WORK at rest, in its face: idle a
sparse line sagging in its middle, waiting a taut solid one, failed cut
in two, running a line the display sets moving."
  (propertize (pcase work
                ('idle (apply #'string (mapcar (lambda (i)
                                                 (let ((middle (and (> i 0) (< i (1- cols)))))
                                                   (if (cl-evenp i) (if middle ?⠠ ?⠄) ?⠀)))
                                               (number-sequence 0 (1- cols)))))
                ('waiting (make-string cols ?⠶))
                ('failed (if (< cols 3) (make-string cols ?⠤)
                           (let* ((gap (max 1 (/ cols 6))) (left (- (/ cols 2) gap)))
                             (concat (make-string left ?⠤) (make-string gap ?⠀)
                                     (make-string (- cols left gap) ?⠤)))))
                (_ (make-string cols ?⠤)))
              'face (romacs-working--face work)))

(defun romacs-working--paint (thread)
  "Write THREAD's still frame over its cells, in place: every braille
character replaced where it stands, so no marker moves and the text
keeps its other properties.  A character that is no braille is not
the thread's, and is left."
  (let* ((buffer (plist-get thread :buffer))
         (start (marker-position (plist-get thread :start)))
         (still (romacs-working-still (plist-get thread :cols) (plist-get thread :work))))
    (when (and start (buffer-live-p buffer))
      (with-current-buffer buffer
        (with-silent-modifications
          (let ((inhibit-read-only t))
            (dotimes (i (min (length still) (- (save-excursion (goto-char start) (line-end-position)) start)))
              (let* ((at (+ start i)) (old (char-after at)) (new (aref still i)))
                (when (and old (<= #x2800 old #x28ff))
                  (unless (eq old new) (subst-char-in-region at (1+ at) old new t))
                  (put-text-property at (1+ at) 'face (get-text-property i 'face still)))))))))))

(defun romacs-working--element (id)
  "What the display draws of thread ID: its cells from its marker, which
the display reads as it freezes each frame, its state, colour and beats."
  (when-let* ((thread (gethash id romacs-working--threads))
              ((marker-position (plist-get thread :start))))
    (list :start (plist-get thread :start) :cols (plist-get thread :cols) :rows 1
          :kind (list 'working (plist-get thread :work) 0
                      (romacs-working--face (plist-get thread :work)) (plist-get thread :beats)))))

(defun romacs-working--animate (id thread)
  "Let the display draw THREAD's motion while its work moves, its motion
going on from one moving state to the other; at rest, nothing is
registered for it."
  (cond ((not (romacs-working--moving-p (plist-get thread :work)))
         (plist-put thread :animated nil)
         (romacs-animate-stop id))
        ((plist-get thread :animated) (romacs-animate-refresh id))
        (t (plist-put thread :animated t)
           (romacs-animate-start id (plist-get thread :buffer) romacs-working-fps
                                 (lambda (_dt) (romacs-working--paint thread))
                                 (lambda (_window) (romacs-working--element id))))))

(defun romacs-working-show (id buffer start cols &optional work)
  "Show thread ID over the COLS characters from START, a line's start in
BUFFER (a marker, followed as it moves), its work WORK (`idle' by
default).  A thread ID shown already moves there."
  (romacs-working-stop id)
  (let ((thread (list :buffer buffer
                      :start (if (markerp start) start (with-current-buffer buffer (copy-marker start)))
                      :own (not (markerp start)) :cols cols :work (or work 'idle) :beats 0)))
    (puthash id thread romacs-working--threads)
    (with-current-buffer buffer
      (add-hook 'kill-buffer-hook
                (lambda () (when (eq (gethash id romacs-working--threads) thread) (romacs-working-stop id)))
                nil t))
    (romacs-working--paint thread)
    (romacs-working--animate id thread))
  id)

(defun romacs-working-set (id work)
  "Thread ID's work goes as WORK now: `idle', `running', `waiting' or `failed'."
  (when-let* ((thread (gethash id romacs-working--threads))
              ((not (eq (plist-get thread :work) work))))
    (plist-put thread :work work)
    (romacs-working--paint thread)
    (romacs-working--animate id thread)))

(defun romacs-working-advance (id &optional n)
  "Thread ID's work made N pieces of progress (one by default): its bead
steps on as many cells."
  (when-let* ((thread (gethash id romacs-working--threads))
              ((eq (plist-get thread :work) 'running))
              ((> (or n 1) 0)))
    (plist-put thread :beats (+ (plist-get thread :beats) (or n 1)))
    (romacs-animate-refresh id)))

(defun romacs-working-stop (id)
  "Show thread ID no more."
  (when-let* ((thread (gethash id romacs-working--threads)))
    (when (plist-get thread :own) (set-marker (plist-get thread :start) nil))
    (remhash id romacs-working--threads)
    (romacs-animate-stop id)))

(provide 'romacs-working)
;;; romacs-working.el ends here
