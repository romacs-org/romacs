;;; romacs-layer.el --- Buffers shown in a layer over the frame  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A layer is a child frame (GNU's `parent-frame') showing one buffer over
;; the root frame.  Showing, moving or hiding it changes no window of the
;; frame below and no terminal's size (DIVERGENCES.md VM2-CHILD-FRAME).
;; Where a layer goes is policy, here: a full-width strip directly above
;; the minibuffer line (or the frame's bottom while the minibuffer is in
;; its own layer) by default, a centred box, or below point.  A
;; layer's frame is kept for its buffer's next show and deleted once the
;; buffer is killed.
;; `romacs-layers' lists what is shown, for agents and tests.  Romacs's
;; menus over layers are romacs-popup.el's.

;;; Code:

(eval-when-compile (require 'cl-lib))
;; The chrome faces, and where a layer ends (`romacs-chrome-bottom').
(require 'romacs-chrome)

(defgroup romacs-layer nil
  "Buffers shown in a layer over the frame."
  :group 'frames)

(defcustom romacs-layer-position 'bottom
  "Where `romacs-layer-show' puts a layer unless told otherwise.
`bottom': the frame's full width, directly above the minibuffer line.
`center': a box centred on the frame.
`point': below point in the selected window.
A caller may also give `romacs-layer-show' (at WINDOW POS): after POS on
its row in WINDOW; or (window-bottom WINDOW): at the bottom left of its text."
  :type '(choice (const bottom) (const center) (const point)))

(defcustom romacs-layer-animation t
  "How the display animates layers.
t: a layer fades in with a short slide from the edge it is anchored
at, fades out, tweens its height when it grows or shrinks, and its
selection highlight (the `romacs-layer-selection' frame parameter)
glides between rows.  `reduced': the fades only.  nil: no animation.
Animation is the display's alone: Lisp sees each layer's final
geometry at once, and a key ends any animation in flight.  A display
on a software renderer (headless test displays) draws none unless the
environment variable ROMACS_LAYER_ANIMATION is 1."
  :type '(choice (const :tag "Fades, slides and tweens" t)
                 (const :tag "Fades only" reduced)
                 (const :tag "None" nil)))

(defcustom romacs-layer-max-lines 12
  "Most lines a layer shows; its buffer scrolls past them."
  :type 'natnum)

(defcustom romacs-layer-frame-parameters
  '((no-accept-focus . t) (no-focus-on-map . t) (min-width . t) (min-height . t)
    (child-frame-border-width . 1) (undecorated . t) (unsplittable . t)
    (no-other-frame . t) (cursor-type . nil) (desktop-dont-save . t)
    (menu-bar-lines . 0) (tool-bar-lines . 0) (tab-bar-lines . 0)
    (left-fringe . 0) (right-fringe . 0) (vertical-scroll-bars . nil))
  "Frame parameters of a layer's child frame."
  :type '(alist :key-type symbol :value-type sexp))

(defcustom romacs-layer-glass t
  "Whether raised layers (the completion panel, message toasts) are glass:
rounded, padded, on `romacs-layer-glass's background,
casting a soft shadow and glowing in `romacs-layer-glow'.  nil gives
them GNU's square child-frame border."
  :type 'boolean)

(defun romacs-layer-glass-parameters (glass &optional face padding radius)
  "Frame parameters that make a layer glass when GLASS is non-nil, else
the ones that make it GNU's square child frame again.  The host rounds a
glass layer's corners (`romacs-layer-radius', RADIUS pixels, default 8),
lights it (`romacs-layer-shadow'), leaves its border, there PADDING
pixels (default 10), unpainted, and paints it in faces: its background
FACE's (default `romacs-layer-glass'), its glow `romacs-layer-glow''s
foreground."
  (if glass
      `((romacs-layer-radius . ,(or radius 8)) (romacs-layer-shadow . t)
        (romacs-layer-face . ,(or face 'romacs-layer-glass))
        (child-frame-border-width . ,(or padding 10)))
    `((romacs-layer-radius) (romacs-layer-shadow) (romacs-layer-face)
      (child-frame-border-width . ,(alist-get 'child-frame-border-width romacs-layer-frame-parameters)))))

(defvar romacs-layer--frames (make-hash-table :test 'eq)
  "BUFFER -> FRAME: the child frame showing each layer's buffer.  A
buffer's `kill-buffer-hook' deletes its layer and its entry.")

(defun romacs-layer--root-frame ()
  "The frame layers are shown over: the selected frame's top-level ancestor."
  (let ((frame (selected-frame)))
    (while (frame-parent frame) (setq frame (frame-parent frame)))
    frame))

(defun romacs-layer--forget ()
  "Delete the current buffer's layer, as the buffer is killed."
  (let ((frame (gethash (current-buffer) romacs-layer--frames)))
    (remhash (current-buffer) romacs-layer--frames)
    (when (frame-live-p frame) (delete-frame frame))))

;; A major mode change keeps it: the layer is the buffer's until it dies.
(put 'romacs-layer--forget 'permanent-local-hook t)

(defun romacs-layer--frame-of (buffer)
  "The live child frame showing BUFFER's layer, if it has one."
  (let ((frame (gethash buffer romacs-layer--frames)))
    (and (frame-live-p frame) frame)))

(defun romacs-layer--frame (buffer)
  "A live child frame for BUFFER, made once and kept for reuse."
  (or (romacs-layer--frame-of buffer)
      (let ((frame (let ((after-make-frame-functions nil))
                     (make-frame `((parent-frame . ,(romacs-layer--root-frame))
                                   (minibuffer . ,(minibuffer-window))
                                   (visibility . nil)
                                   ,@romacs-layer-frame-parameters)))))
        (puthash buffer frame romacs-layer--frames)
        (with-current-buffer buffer (add-hook 'kill-buffer-hook #'romacs-layer--forget nil t))
        (let ((window (frame-root-window frame)))
          (set-window-buffer window buffer)
          (set-window-dedicated-p window t)
          ;; Ordinary window cycling skips it (window.el `other-window').
          (set-window-parameter window 'no-other-window t)
          (set-window-parameter window 'no-delete-other-windows t))
        frame)))

(defun romacs-layer--text-size (buffer)
  "BUFFER's text in columns and lines, as (COLUMNS . LINES)."
  (with-current-buffer buffer
    (save-excursion
      (goto-char (point-min))
      (let ((columns 0) (lines 0))
        (while (not (eobp))
          (setq columns (max columns (string-width (buffer-substring (point) (line-end-position))))
                lines (1+ lines))
          (forward-line 1))
        (cons (max columns 1) (max lines 1))))))

(declare-function romacs-chrome-bottom "romacs-chrome" (&optional frame except))
(declare-function romacs-chrome-top "romacs-chrome" (&optional frame))

(defun romacs-layer--bottom (root)
  "Where a `bottom' layer of ROOT ends: above the status bar and the
minibuffer line, or the layer hosting the minibuffer window
\(`romacs-chrome-bottom')."
  (romacs-chrome-bottom root))

(defun romacs-layer--placement (position columns lines border)
  "Where POSITION puts a layer of COLUMNS by LINES with BORDER pixels:
\(LEFT TOP WIDTH HEIGHT), its outer position in the root frame and its
text size, in pixels."
  (let* ((root (romacs-layer--root-frame))
         (cw (frame-char-width root)) (ch (frame-char-height root))
         ;; Child frames are placed from the native origin, outside the
         ;; internal border round the windows.
         (inset (frame-internal-border-width root))
         (frame-width (- (frame-text-width root) (* 2 border)))
         (width (min (* columns cw) frame-width))
         (height (* lines ch))
         (outer-height (+ height (* 2 border))))
    (pcase position
      ('bottom
       (let ((top (- (romacs-layer--bottom root) outer-height)))
         (list inset (max inset top) frame-width height)))
      ('center
       (let ((width (min width (- frame-width (* 4 cw)))))
         (list (+ inset (/ (- frame-width width) 2)) (+ inset (/ (- (frame-text-height root) outer-height) 2)) width height)))
      (`(at ,window ,pos)
       (romacs-layer--after window pos width height border))
      (`(window-bottom ,window)
       ;; At the bottom left of WINDOW's text, a margin inside it.
       (pcase-let ((`(,left ,_ ,right ,bottom) (window-edges window t nil t)))
         (let ((width (min width (- right left 24 (* 2 border)))))
           (list (+ left 12) (- bottom 12 outer-height) (max cw width) height))))
      ('point
       (let* ((edges (window-edges nil t nil t))
              (posn (posn-x-y (posn-at-point)))
              (x (+ (nth 0 edges) (or (car posn) 0)))
              (y (+ (nth 1 edges) (or (cdr posn) 0) ch)))
         (list (min x (max 0 (- frame-width width)))
               (if (> (+ y outer-height) (frame-text-height root)) (max 0 (- y ch outer-height)) y)
               width height)))
      (_ (error "Unknown layer position %S" position)))))

(defun romacs-layer--after (window pos width height border)
  "Where a layer of WIDTH by HEIGHT text pixels inside BORDER goes after
POS in WINDOW: one column after POS's glyph, its text on POS's row; on
the row below, flush with the window's right edge, when it would cross
that edge; on the row above when the row below is past the window's
bottom.  As (LEFT TOP WIDTH HEIGHT), or nil when POS is not shown."
  (when-let* ((posn (posn-at-point pos window))
              (xy (posn-x-y posn)))
    (let* ((edges (window-edges window t nil t))
           (left (nth 0 edges)) (top (nth 1 edges)) (right (nth 2 edges)) (bottom (nth 3 edges))
           (ch (frame-char-height (window-frame window)))
           (outer (+ width (* 2 border)))
           (x (+ left (car xy) (frame-char-width (window-frame window))))
           (y (+ top (cdr xy))))
      (cond
       ((<= (+ x outer) right))
       ((<= (+ y ch height) bottom) (setq x (max left (- right outer)) y (+ y ch)))
       (t (setq x (max left (- right outer)) y (max top (- y height)))))
      (list x (- y border) width height))))

(cl-defun romacs-layer-show (buffer &key position columns lines max-lines)
  "Show BUFFER in a layer over the frame and return the layer's frame.
POSITION is `bottom', `center', `point' or (at WINDOW POS),
`romacs-layer-position' by default; nil, showing nothing, when POS is
not shown in WINDOW.  COLUMNS and LINES size it, by default to BUFFER's text, at
most MAX-LINES lines, by default `romacs-layer-max-lines'; a `bottom' layer is as wide as the
frame."
  (let* ((buffer (window-normalize-buffer buffer))
         (frame (romacs-layer--frame buffer))
         (text (romacs-layer--text-size buffer))
         (border (or (frame-parameter frame 'child-frame-border-width) 0)))
    (pcase (romacs-layer--placement (or position romacs-layer-position)
                                    (or columns (car text))
                                    (min (or lines (cdr text)) (or max-lines romacs-layer-max-lines))
                                    border)
      (`(,left ,top ,width ,height)
       (set-frame-size frame width height t)
       (set-frame-position frame left top)
       ;; The edge it slides in from and grows away from.
       (set-frame-parameter frame 'romacs-layer-edge
                            (if (eq (or position romacs-layer-position) 'bottom) 'bottom 'top))
       ;; A layer shown anew starts at its text's top; one re-placed while
       ;; shown keeps its viewport (a scrolled popup refitting).
       (unless (frame-visible-p frame)
         (set-window-start (frame-root-window frame) (with-current-buffer buffer (point-min))))
       (make-frame-visible frame)
       frame)
      (_ (romacs-layer-hide buffer) nil))))

(defun romacs-layer-hide (buffer)
  "Hide the layer showing BUFFER, keeping its frame for the next show."
  (let ((frame (and (get-buffer buffer) (romacs-layer--frame-of (get-buffer buffer)))))
    (when (and frame (frame-visible-p frame)) (make-frame-invisible frame))))

(defun romacs-layers ()
  "The child frames shown over the frame, oldest first: for each, a plist
of its buffer, its outer position in its parent, and its text size in
columns and lines."
  (cl-loop for frame in (visible-frame-list)
           when (frame-parent frame)
           collect (list :frame frame
                         :buffer (window-buffer (frame-root-window frame))
                         :position (frame-position frame)
                         :size (cons (frame-text-cols frame) (frame-text-lines frame))
                         :minibuffer (eq frame (romacs--minibuffer-host)))))

;; Messages as toasts: a stack from the top-right corner of the selected
;; window's text (or the frame's, `romacs-layer-toast-position'), which
;; covers no status bar and no minibuffer.  The echo area's semantics are
;; GNU's (`message', `current-message', *Messages*); only where a message
;; shows is ours (DIVERGENCES.md VM2-ECHO-LAYER).

(defcustom romacs-layer-toast-timeout 2.5
  "Seconds a message toast shows before it fades, unless the next command
clears the echo area first."
  :type 'number)

(defcustom romacs-layer-toast-max 3
  "Most message toasts shown at once; the oldest goes first."
  :type 'natnum)

(defcustom romacs-layer-toast-position 'window
  "Where message toasts stack.
`window': from the top-right corner of the selected window's text, inside
its pane, following the selection and the window's size.
`frame': from the frame's top-right corner, below `romacs-chrome-top'."
  :type '(choice (const window) (const frame)))

(defvar romacs-layer-echo-claim-functions nil
  "Functions offered the echo area's new message before it shows as a
toast, each called with the text; one returning non-nil has shown it
otherwise (an eval chip, romacs-eval-chip.el), and no toast shows.")

(defvar romacs-layer--toasts nil
  "The toasts shown, newest first: each (BUFFER TEXT TIMER).")

(defun romacs-layer--toast-buffer ()
  "A toast buffer no shown toast uses."
  (let ((used (mapcar #'car romacs-layer--toasts)) (n 0) buffer)
    (while (memq (setq buffer (get-buffer-create (format " *romacs-toast-%d*" n))) used)
      (setq n (1+ n)))
    (with-current-buffer buffer
      (setq-local mode-line-format nil header-line-format nil cursor-type nil
                  truncate-lines nil word-wrap t))
    buffer))

(defun romacs-layer--toast-size (text columns)
  "TEXT's size in a toast at most COLUMNS wide: (COLUMNS . LINES)."
  (let* ((lines (split-string text "\n"))
         (width (min columns (max 1 (apply #'max (mapcar #'string-width lines))))))
    (cons width (min 6 (apply #'+ (mapcar (lambda (l) (max 1 (ceiling (string-width l) (float width)))) lines))))))

(defun romacs-layer--toast-window ()
  "The window toasts show in: the root frame's selected window, or the
window a prompt was entered from while the minibuffer is selected."
  (let ((window (frame-selected-window (romacs-layer--root-frame))))
    (if (window-minibuffer-p window)
        (or (minibuffer-selected-window) (get-mru-window (romacs-layer--root-frame) nil t))
      window)))

(defcustom romacs-layer-toast-min-columns 16
  "Fewest text columns a toast in the selected window has; a narrower
window's toasts stack in the frame's corner."
  :type 'natnum)

(defconst romacs-layer--toast-padding 10
  "A glass toast's padding: its `child-frame-border-width'.")

(defvar romacs-layer--toast-placed nil
  "What the toasts were last stacked for: the area and the toasts.")

(defun romacs-layer--toast-area ()
  "Where toasts stack, in the root frame's native pixels: (LEFT TOP RIGHT
BOTTOM COLUMNS), COLUMNS the most a toast's text is wide.  The selected
window's text area for `window' (`romacs-layer-toast-position'), else
the frame's, below the chrome above it and above the chrome below it.
A window too narrow for a toast of `romacs-layer-toast-min-columns' with
its padding and margin inside it takes the frame's area instead."
  (let* ((root (romacs-layer--root-frame))
         (cw (frame-char-width root))
         (window (and (eq romacs-layer-toast-position 'window) (romacs-layer--toast-window)))
         (third (max 20 (/ (frame-text-cols root) 3)))
         ;; The columns an area of PIXELS holds for a toast's text: less
         ;; the margin, the glass padding either side and the column a
         ;; window's last keeps for its continuation glyph.
         (columns (lambda (pixels) (1- (/ (- pixels 12 (* 2 romacs-layer--toast-padding)) cw)))))
    (or (and (window-live-p window)
             (pcase-let ((`(,left ,top ,right ,bottom) (window-edges window t nil t)))
               (let ((room (funcall columns (- right left))))
                 (and (>= room romacs-layer-toast-min-columns)
                      (list left top right (min bottom (romacs-chrome-bottom root)) (min third room))))))
        (list 0 (romacs-chrome-top root) (- (frame-native-width root) (frame-internal-border-width root))
              (romacs-chrome-bottom root) third))))

(defun romacs-layer--toast-follow (frame)
  "Restack the toasts after a change to FRAME's windows (the selection, a
size), when FRAME is the root frame and where they go moved."
  (when (and romacs-layer--toasts (eq frame (romacs-layer--root-frame))
             (not (equal romacs-layer--toast-placed (cons (romacs-layer--toast-area) romacs-layer--toasts))))
    (romacs-layer--toast-stack)))

(defun romacs-layer--toast-hooks (on)
  "Follow window changes while toasts show (ON), else not."
  (dolist (hook '(window-selection-change-functions window-size-change-functions))
    (if on (add-hook hook #'romacs-layer--toast-follow) (remove-hook hook #'romacs-layer--toast-follow))))

(defun romacs-layer--toast-stack ()
  "Place the shown toasts from the top right of `romacs-layer--toast-area'
downward, newest on top; a toast that would pass its bottom waits hidden."
  (pcase-let* ((root (romacs-layer--root-frame))
               (cw (frame-char-width root)) (ch (frame-char-height root))
               (`(,_ ,area-top ,right ,bottom ,columns) (romacs-layer--toast-area))
               (margin 12) (gap 8) (top (+ area-top margin)))
    (romacs-layer--toast-hooks romacs-layer--toasts)
    (setq romacs-layer--toast-placed (cons (romacs-layer--toast-area) romacs-layer--toasts))
    (dolist (toast romacs-layer--toasts)
      (let* ((frame (romacs-layer--frame (car toast)))
             (border (or (frame-parameter frame 'child-frame-border-width) 0))
             (size (romacs-layer--toast-size (nth 1 toast) columns))
             ;; One column more: a window's last holds its continuation glyph.
             (width (* (1+ (car size)) cw)) (height (* (cdr size) ch)))
        (set-frame-size frame width height t)
        (set-frame-position frame (- right width (* 2 border) margin) top)
        (set-frame-parameter frame 'romacs-layer-edge 'top)
        (if (> (+ top height (* 2 border)) bottom)
            (when (frame-visible-p frame) (make-frame-invisible frame))
          ;; A new toast shows above every other layer, the panel too.
          (unless (frame-visible-p frame) (make-frame-visible frame) (raise-frame frame)))
        (setq top (+ top height (* 2 border) gap))))))

(defun romacs-layer--toast-expire (buffer)
  "Hide the toast showing BUFFER and close up the stack."
  (setq romacs-layer--toasts (seq-remove (lambda (toast) (eq (car toast) buffer)) romacs-layer--toasts))
  (romacs-layer-hide buffer)
  (romacs-layer--toast-stack))

(defun romacs-layer-toast-clear ()
  "Hide every toast, which fades, as the echo area clears.  Nothing to do
when none shows: the echo area clears as every command starts."
  (when romacs-layer--toasts
    (let ((toasts romacs-layer--toasts))
      (setq romacs-layer--toasts nil romacs-layer--toast-placed nil)
      (romacs-layer--toast-hooks nil)
      (dolist (toast toasts)
        (cancel-timer (nth 2 toast))
        (romacs-layer-hide (car toast))))))

(defun romacs-layer-toast (text)
  "Show TEXT as a toast for `romacs-layer-toast-timeout' seconds.
The same TEXT as the newest toast shows that one longer instead."
  (if (equal text (nth 1 (car romacs-layer--toasts)))
      (timer-set-time (nth 2 (car romacs-layer--toasts))
                      (timer-relative-time nil romacs-layer-toast-timeout))
    (while (>= (length romacs-layer--toasts) (max 1 romacs-layer-toast-max))
      (let ((oldest (car (last romacs-layer--toasts))))
        (cancel-timer (nth 2 oldest))
        (romacs-layer--toast-expire (car oldest))))
    (let ((buffer (romacs-layer--toast-buffer)))
      (with-current-buffer buffer
        (let ((inhibit-read-only t)) (erase-buffer) (insert text)))
      (let ((frame (romacs-layer--frame buffer)))
        (modify-frame-parameters frame (romacs-layer-glass-parameters romacs-layer-glass 'romacs-layer-toast))
        (set-window-start (frame-root-window frame) 1))
      (push (list buffer text (run-at-time romacs-layer-toast-timeout nil #'romacs-layer--toast-expire buffer))
            romacs-layer--toasts)
      (romacs-layer--toast-stack))))

(provide 'romacs-layer)
;;; romacs-layer.el ends here
