;;; romacs-animate.el --- Frame-by-frame animation in buffers  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Animations a buffer shows in its own text, with one scheduler:
;;
;;   (romacs-animate-start ID BUFFER FPS FRAME-FN &optional ELEMENT-FN)
;;
;; FRAME-FN runs with BUFFER current and one argument, DT: the seconds of
;; motion since its last frame (0 for a still frame, e.g. the first one or
;; one after a resize).  It reads the size of BUFFER's window itself.
;;
;; With ELEMENT-FN the display draws the motion itself, frame after frame,
;; over the text FRAME-FN left: no Lisp runs per frame.  ELEMENT-FN runs
;; with BUFFER current and the window showing it, and returns what the
;; display draws there, a plist of :top, :cols, :rows and :kind, or nil
;; for nothing.  The scheduler keeps it in `romacs-animate--elements',
;; which every frame the evaluator freezes carries to the display while
;; the window shows BUFFER, and FRAME-FN draws only still frames.
;; Without ELEMENT-FN, or where the display draws no
;; animation, FRAME-FN draws every frame from one Lisp frame clock: a
;; repeating timer at the fastest rate a moving animation asks for.
;;
;; Either way an animation moves only while its buffer is shown in the
;; selected frame, that frame has focus, it is not paused, and motion is
;; allowed (`romacs-animate-motion-p').  Otherwise nothing runs for it:
;; there is no timer, and the display holds its last frame.  A command, a
;; focus change or `romacs-animate-redraw' looks again; the display itself
;; also stops the moment its window loses focus or is hidden.  Where motion
;; is not allowed an animation still gets a still frame when it becomes
;; visible or its window resizes.
;;
;; `romacs-animate-statistics' reports the evaluator time FRAME-FNs take.
;; rlisp/romacs-landing.el, the *Romacs* landing, is the first client.

;;; Code:

(defvar romacs-layer-animation)         ; rlisp/romacs-layer.el

(defvar romacs-animate-max-dt 0.08
  "Most seconds of motion one frame advances, however long it was late.")

(defun romacs-animate--slot (animation slot)
  "ANIMATION's SLOT: one of :id, :key, :buffer, :fps, :fn, :element,
:paused, :last and :size."
  (plist-get (cdr animation) slot))

(defun romacs-animate--set (animation slot value)
  (setcdr animation (plist-put (cdr animation) slot value))
  value)

(defvar romacs-animate--animations nil
  "The registered animations, newest first.")

(defvar romacs-animate--keys 0
  "The last key an animation was given on the display.")

(defvar romacs-animate--retry nil
  "A wake due while the display has not yet said whether it animates.")

(defvar romacs-animate--timer nil
  "The Lisp frame clock, or nil while no Lisp-drawn animation moves.")

(defvar romacs-animate--interval nil
  "The Lisp frame clock's period in seconds.")

(defvar romacs-animate--elements nil
  "The text animations the display draws by itself, a list of (KEY WINDOW
BUFFER TOP COLS ROWS FPS MOVING KIND), one for each window that
shows an animation, as `romacs-animate--sync' keeps it.  Each frame the
evaluator freezes carries those whose WINDOW still shows BUFFER
\(`Vm::text_animations' in the VM), so the display draws exactly the
animations of the frame it shows.")

(defvar romacs-animate--statistics (vector 0 0.0 0.0)
  "Frames FRAME-FNs drew, their total and their longest evaluator seconds.")

(defvar romacs-animate--display nil
  "Whether the display animates, as it last said (`romacs--display-animates').")

(defun romacs-animate-motion-p ()
  "Whether animations move here.
Not in batch, nor where `romacs-layer-animation' is nil or `reduced',
nor on a display that draws no animation (`romacs--display-animates':
a software rasterizer, every headless harness's, unless the environment
variable ROMACS_LAYER_ANIMATION is 1).  While the display cannot say,
its last answer stands.  A void `romacs-layer-animation' counts as its
default, t."
  (and (not noninteractive)
       (eq (if (boundp 'romacs-layer-animation) romacs-layer-animation t) t)
       (fboundp 'romacs--display-animates)
       (let ((animates (romacs--display-animates)))
         (unless (eq animates 'unknown) (setq romacs-animate--display animates))
         (eq romacs-animate--display t))))

(defun romacs-animate--get (id)
  (let ((found nil))
    (dolist (animation romacs-animate--animations found)
      (when (equal (romacs-animate--slot animation :id) id) (setq found animation)))))

(defun romacs-animate--windows (animation)
  "The windows of the selected frame that show ANIMATION's buffer: each
shows its own animation, whichever window is selected."
  (let ((buffer (romacs-animate--slot animation :buffer)))
    (and (buffer-live-p buffer) (get-buffer-window-list buffer 'nomini (selected-frame)))))

(defun romacs-animate--focused-p ()
  "Whether the selected frame has focus, or no focus change reached it yet."
  (let ((update (assq 'last-focus-update (frame-parameters))))
    (or (null update) (cdr update))))

(defun romacs-animate--moving-p (animation motion)
  "Whether ANIMATION moves: MOTION allowed, not paused, shown in the
selected frame, and that frame focused.  Which window is selected does
not matter."
  (and motion (not (romacs-animate--slot animation :paused))
       (romacs-animate--windows animation) (romacs-animate--focused-p)))

(defun romacs-animate--failed (animation err)
  (romacs-animate-stop (romacs-animate--slot animation :id))
  (message "Animation %S stopped: %s" (romacs-animate--slot animation :id) (error-message-string err)))

(defun romacs-animate--draw (animation dt)
  "Run ANIMATION's frame function with DT, timed; a failing one is stopped."
  (let ((start (float-time)))
    (condition-case err
        (with-current-buffer (romacs-animate--slot animation :buffer)
          (funcall (romacs-animate--slot animation :fn) dt))
      (error (romacs-animate--failed animation err)))
    (let ((seconds (- (float-time) start))
          (stats romacs-animate--statistics))
      (aset stats 0 (1+ (aref stats 0)))
      (aset stats 1 (+ (aref stats 1) seconds))
      (aset stats 2 (max (aref stats 2) seconds)))))

(defun romacs-animate--size (animation)
  "The sizes of the windows that show ANIMATION's buffer, smallest first:
whichever of them is selected, the same list."
  (sort (mapcar (lambda (window) (cons (window-body-width window) (window-body-height window)))
                (romacs-animate--windows animation))
        (lambda (a b) (or (< (car a) (car b)) (and (= (car a) (car b)) (< (cdr a) (cdr b)))))))

(defun romacs-animate--still (animation)
  "Give ANIMATION a still frame if it is visible and its windows' sizes
are not the ones its last frame was drawn for."
  (let ((size (romacs-animate--size animation)))
    (when (and size (not (equal size (romacs-animate--slot animation :size))))
      (romacs-animate--set animation :size size)
      (romacs-animate--draw animation 0))))

(defun romacs-animate--hosted-p (animation motion)
  "Whether the display draws ANIMATION's motion: it has an element and
MOTION is allowed."
  (and motion (romacs-animate--slot animation :element)))

(defun romacs-animate--sync (animation motion)
  "Make ANIMATION's entries in `romacs-animate--elements' its elements as
they now are: one for each window that shows it where the display draws
it, else none."
  (let* ((key (romacs-animate--slot animation :key))
         (buffer (romacs-animate--slot animation :buffer))
         (moving (and (romacs-animate--moving-p animation motion) t))
         (entries nil))
    (when (romacs-animate--hosted-p animation motion)
      (dolist (window (romacs-animate--windows animation))
        (let ((element (with-current-buffer buffer
                         (funcall (romacs-animate--slot animation :element) window))))
          (when element
            (push (list key window buffer (plist-get element :top) (plist-get element :cols)
                        (plist-get element :rows) (romacs-animate--slot animation :fps) moving
                        (plist-get element :kind))
                  entries)))))
    (setq romacs-animate--elements (nconc (nreverse entries) (romacs-animate--without key)))))

(defun romacs-animate--without (key)
  "`romacs-animate--elements' without KEY's entry."
  (let ((kept nil))
    (dolist (entry romacs-animate--elements (nreverse kept))
      (unless (eq (car entry) key) (push entry kept)))))

(defun romacs-animate--tick ()
  "The Lisp frame clock: draw each moving animation the display does not
draw whose period has passed; stop the clock when none moves."
  (let ((now (float-time))
        (motion (romacs-animate-motion-p))
        (moving nil))
    (dolist (animation romacs-animate--animations)
      (if (or (romacs-animate--hosted-p animation motion)
              (not (romacs-animate--moving-p animation motion)))
          (romacs-animate--set animation :last nil)
        (setq moving t)
        (let* ((last (romacs-animate--slot animation :last))
               (period (/ 1.0 (romacs-animate--slot animation :fps)))
               (elapsed (and last (- now last))))
          ;; A tick a little early for a slower animation still counts.
          (when (or (null elapsed) (>= elapsed (* 0.9 period)))
            (romacs-animate--set animation :last now)
            (romacs-animate--set animation :size (romacs-animate--size animation))
            (romacs-animate--draw animation (if elapsed (min elapsed romacs-animate-max-dt) 0))))))
    (unless moving (romacs-animate--clock nil))))

(defun romacs-animate--clock (interval)
  "Run the Lisp frame clock every INTERVAL seconds, or stop it when nil."
  (unless (equal interval romacs-animate--interval)
    (when romacs-animate--timer (cancel-timer romacs-animate--timer))
    (setq romacs-animate--timer (and interval (run-at-time interval interval #'romacs-animate--tick))
          romacs-animate--interval interval)))

(defun romacs-animate--wake (&rest _)
  "Draw still frames that are due, bring the display's elements up to
date, and run the Lisp clock if a Lisp-drawn animation moves.  On
`post-command-hook', `window-size-change-functions' and
`after-focus-change-function' while any animation is registered."
  (let ((motion (romacs-animate-motion-p))
        (fps nil))
    (dolist (animation romacs-animate--animations)
      (if (and (romacs-animate--moving-p animation motion)
               (not (romacs-animate--hosted-p animation motion)))
          (setq fps (max (or fps 0) (romacs-animate--slot animation :fps)))
        ;; Stopped: its next motion starts from a still frame.
        (romacs-animate--set animation :last nil)
        (romacs-animate--still animation)
        (romacs-animate--sync animation motion)))
    (romacs-animate--clock (and fps (/ 1.0 fps)))
    ;; Until its first paint the window does not know its adapter.
    (when (and romacs-animate--animations (not romacs-animate--retry)
               (fboundp 'romacs--display-animates) (eq (romacs--display-animates) 'unknown))
      (setq romacs-animate--retry
            (run-at-time 0.25 nil (lambda () (setq romacs-animate--retry nil) (romacs-animate--wake)))))))

(defun romacs-animate--watch (on)
  "Wake on commands and focus changes when ON, else not."
  (if on
      (progn (add-hook 'post-command-hook #'romacs-animate--wake)
             (add-hook 'window-size-change-functions #'romacs-animate--wake)
             (add-function :after after-focus-change-function #'romacs-animate--wake))
    (remove-hook 'post-command-hook #'romacs-animate--wake)
    (remove-hook 'window-size-change-functions #'romacs-animate--wake)
    (remove-function after-focus-change-function #'romacs-animate--wake)))

(defun romacs-animate-start (id buffer fps frame-fn &optional element-fn)
  "Animate BUFFER as ID at up to FPS frames a second with FRAME-FN.
FRAME-FN is called with BUFFER current and the seconds of motion since
its last frame, 0 for a still frame.  With ELEMENT-FN the display draws
the motion over FRAME-FN's still frames: called with BUFFER current and
the window showing it, it returns the display's element, a plist of
:top, :cols, :rows and :kind (see `romacs-animate--elements'),
or nil.  A running ID is replaced.  The first frame comes once BUFFER is
shown; ID stops when BUFFER is killed."
  (romacs-animate-stop id)
  (let ((animation (list 'romacs-animate :id id :key (setq romacs-animate--keys (1+ romacs-animate--keys))
                         :buffer buffer :fps fps :fn frame-fn :element element-fn)))
    (push animation romacs-animate--animations)
    (with-current-buffer buffer
      ;; Only this registration: a later one of ID may animate elsewhere.
      (add-hook 'kill-buffer-hook
                (lambda () (when (eq (romacs-animate--get id) animation) (romacs-animate-stop id)))
                nil t)))
  (romacs-animate--watch t)
  (run-at-time 0 nil #'romacs-animate--wake)
  id)

(defun romacs-animate-stop (id)
  "Stop and forget animation ID."
  (let ((animation (romacs-animate--get id)))
    (when animation
      (setq romacs-animate--elements (romacs-animate--without (romacs-animate--slot animation :key)))
      (setq romacs-animate--animations (delq animation romacs-animate--animations))))
  (unless romacs-animate--animations (romacs-animate--watch nil))
  (romacs-animate--wake))

(defun romacs-animate-pause (id)
  "Hold animation ID at its current frame."
  (let ((animation (romacs-animate--get id)))
    (when animation (romacs-animate--set animation :paused t))
    (romacs-animate--wake)))

(defun romacs-animate-resume (id)
  "Let paused animation ID move again."
  (let ((animation (romacs-animate--get id)))
    (when animation (romacs-animate--set animation :paused nil))
    (romacs-animate--wake)))

(defun romacs-animate-paused-p (id)
  "Whether animation ID is paused."
  (let ((animation (romacs-animate--get id)))
    (and animation (romacs-animate--slot animation :paused))))

(defun romacs-animate-redraw (id)
  "Draw a still frame of animation ID now, if it is shown, and give the
display its element as it now is."
  (let ((animation (romacs-animate--get id)))
    (when (and animation (romacs-animate--windows animation))
      (romacs-animate--draw animation 0)
      (romacs-animate--sync animation (romacs-animate-motion-p)))))

(defun romacs-animate-statistics (&optional reset)
  "The evaluator time FRAME-FNs took: a plist of :frames, :seconds
\(their total) and :max-seconds.  Frames the display draws itself take
none and are not counted.  With RESET, start counting again."
  (let ((stats romacs-animate--statistics))
    (prog1 (list :frames (aref stats 0) :seconds (aref stats 1) :max-seconds (aref stats 2))
      (when reset (setq romacs-animate--statistics (vector 0 0.0 0.0))))))

(provide 'romacs-animate)
;;; romacs-animate.el ends here
