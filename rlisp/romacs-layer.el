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
;; `romacs-layers' lists what is shown, for agents and tests.  The
;; which-key functions at the end let `which-key-popup-type' `custom' show
;; its popup in a layer.

;;; Code:

(eval-when-compile (require 'cl-lib))

(defgroup romacs-layer nil
  "Buffers shown in a layer over the frame."
  :group 'frames)

(defcustom romacs-layer-position 'bottom
  "Where `romacs-layer-show' puts a layer unless told otherwise.
`bottom': the frame's full width, directly above the minibuffer line.
`center': a box centred on the frame.
`point': below point in the selected window."
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

(defface romacs-layer-glow `((t :foreground ,romacs-chrome-builtin-accent))
  "The colour a raised layer glows in: the key light of the Ember Lattice
design language, the theme's accent.  Unless a theme sets it, it is
derived from the theme (`romacs-chrome-derived-faces').")

(defcustom romacs-layer-glass t
  "Whether raised layers (the completion panel, message toasts) are glass:
rounded, padded, on the background lifted 8% toward the foreground,
casting a soft shadow and glowing in `romacs-layer-glow'.  nil gives
them GNU's square child-frame border."
  :type 'boolean)

(defun romacs-layer--hex (color)
  "COLOR, a colour name, as #RRGGBB, or nil when it is unknown."
  (when-let* ((values (and color (color-values color))))
    (apply #'format "#%02x%02x%02x" (mapcar (lambda (v) (/ v 257)) values))))

(defun romacs-layer-glass-parameters (glass)
  "Frame parameters that make a layer glass when GLASS is non-nil, else
the ones that make it GNU's square child frame again.  The host rounds a glass layer's corners (`romacs-layer-radius'),
lights it (`romacs-layer-shadow', `romacs-layer-glow') and leaves its
border, there padding, unpainted."
  (if glass
      `((romacs-layer-radius . 8) (romacs-layer-shadow . t)
        (romacs-layer-glow . ,(romacs-layer--hex (face-foreground 'romacs-layer-glow nil t)))
        (child-frame-border-width . 10)
        (background-color . ,(romacs-chrome-mix 0.08)))
    `((romacs-layer-radius) (romacs-layer-shadow) (romacs-layer-glow) (background-color)
      (child-frame-border-width . ,(alist-get 'child-frame-border-width romacs-layer-frame-parameters)))))

(defun romacs-layer-refresh ()
  "Make every glass layer glass again with the faces as they are now:
after a theme change (`romacs-chrome-refresh')."
  (dolist (frame (frame-list))
    (when (and (frame-parent frame) (frame-parameter frame 'romacs-layer-radius))
      (modify-frame-parameters frame (romacs-layer-glass-parameters t)))))

(defvar romacs-layer--frames nil
  "Alist of (BUFFER . FRAME): the child frame showing each layer's buffer.")

(defun romacs-layer--root-frame ()
  "The frame layers are shown over: the selected frame's top-level ancestor."
  (let ((frame (selected-frame)))
    (while (frame-parent frame) (setq frame (frame-parent frame)))
    frame))

(defun romacs-layer--prune ()
  "Delete the layers of killed buffers."
  (dolist (entry romacs-layer--frames)
    (unless (buffer-live-p (car entry))
      (when (frame-live-p (cdr entry)) (delete-frame (cdr entry)))))
  (setq romacs-layer--frames (seq-filter (lambda (entry) (buffer-live-p (car entry))) romacs-layer--frames)))

(defun romacs-layer--frame (buffer)
  "A live child frame for BUFFER, made once and kept for reuse."
  (romacs-layer--prune)
  (let ((frame (cdr (assq buffer romacs-layer--frames))))
    (unless (frame-live-p frame)
      (let ((after-make-frame-functions nil))
        (setq frame (make-frame `((parent-frame . ,(romacs-layer--root-frame))
                                  (minibuffer . ,(minibuffer-window))
                                  (visibility . nil)
                                  ,@romacs-layer-frame-parameters))))
      (setf (alist-get buffer romacs-layer--frames) frame)
      (let ((window (frame-root-window frame)))
        (set-window-buffer window buffer)
        (set-window-dedicated-p window t)
        ;; Ordinary window cycling skips it (window.el `other-window').
        (set-window-parameter window 'no-other-window t)
        (set-window-parameter window 'no-delete-other-windows t)))
    frame))

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

(defun romacs-layer--bottom (root)
  "Where a `bottom' layer of ROOT ends: above the minibuffer line, or at
ROOT's bottom edge when a child frame hosts the minibuffer window
\(`romacs--host-minibuffer')."
  (if (romacs--minibuffer-host)
      (frame-text-height root)
    (window-pixel-top (minibuffer-window root))))

(defun romacs-layer--placement (position columns lines border)
  "Where POSITION puts a layer of COLUMNS by LINES with BORDER pixels:
\(LEFT TOP WIDTH HEIGHT), its outer position in the root frame and its
text size, in pixels."
  (let* ((root (romacs-layer--root-frame))
         (cw (frame-char-width root)) (ch (frame-char-height root))
         (frame-width (- (frame-text-width root) (* 2 border)))
         (width (min (* columns cw) frame-width))
         (height (* lines ch))
         (outer-height (+ height (* 2 border))))
    (pcase position
      ('bottom
       (let ((top (- (romacs-layer--bottom root) outer-height)))
         (list 0 (max 0 top) frame-width height)))
      ('center
       (let ((width (min width (- frame-width (* 4 cw)))))
         (list (/ (- frame-width width) 2) (/ (- (frame-text-height root) outer-height) 2) width height)))
      ('point
       (let* ((edges (window-inside-pixel-edges))
              (posn (posn-x-y (posn-at-point)))
              (x (+ (nth 0 edges) (or (car posn) 0)))
              (y (+ (nth 1 edges) (or (cdr posn) 0) ch)))
         (list (min x (max 0 (- frame-width width)))
               (if (> (+ y outer-height) (frame-text-height root)) (max 0 (- y ch outer-height)) y)
               width height)))
      (_ (error "Unknown layer position %S" position)))))

(cl-defun romacs-layer-show (buffer &key position columns lines)
  "Show BUFFER in a layer over the frame and return the layer's frame.
POSITION is `bottom', `center' or `point', `romacs-layer-position' by
default.  COLUMNS and LINES size it, by default to BUFFER's text, at
most `romacs-layer-max-lines' lines; a `bottom' layer is as wide as the
frame."
  (let* ((buffer (window-normalize-buffer buffer))
         (frame (romacs-layer--frame buffer))
         (text (romacs-layer--text-size buffer))
         (border (or (frame-parameter frame 'child-frame-border-width) 0)))
    (pcase-let ((`(,left ,top ,width ,height)
                 (romacs-layer--placement (or position romacs-layer-position)
                                          (or columns (car text))
                                          (min (or lines (cdr text)) romacs-layer-max-lines)
                                          border)))
      (set-frame-size frame width height t)
      (set-frame-position frame left top)
      ;; The edge it slides in from and grows away from.
      (set-frame-parameter frame 'romacs-layer-edge
                           (if (eq (or position romacs-layer-position) 'bottom) 'bottom 'top)))
    (set-window-start (frame-root-window frame) (with-current-buffer buffer (point-min)))
    (make-frame-visible frame)
    frame))

(defun romacs-layer-hide (buffer)
  "Hide the layer showing BUFFER, keeping its frame for the next show."
  (let ((frame (cdr (assq (get-buffer buffer) romacs-layer--frames))))
    (when (frame-live-p frame) (make-frame-invisible frame))))

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

;; which-key's `custom' popup type (which-key.el `which-key-custom-*').

(defvar which-key--buffer)

(defun romacs-layer-which-key-show (dimensions)
  "Show which-key's buffer in a bottom layer; DIMENSIONS is (LINES . COLUMNS)."
  (romacs-layer-show which-key--buffer :position 'bottom :lines (car dimensions)))

(defun romacs-layer-which-key-hide ()
  "Hide which-key's layer."
  (when (buffer-live-p which-key--buffer)
    (romacs-layer-hide which-key--buffer)))

(defun romacs-layer-which-key-max-dimensions (_selected-window-width)
  "The most which-key's layer shows, as (LINES . COLUMNS)."
  (let ((root (romacs-layer--root-frame)))
    (cons romacs-layer-max-lines
          (- (/ (frame-text-width root) (frame-char-width root)) 2))))

;; Messages as toasts: a stack in the frame's top-right corner, which
;; covers no mode line and no minibuffer.  The echo area's semantics are
;; GNU's (`message', `current-message', *Messages*); only where a message
;; shows is ours (DIVERGENCES.md VM2-ECHO-LAYER).

(defcustom romacs-layer-toast-timeout 4
  "Seconds a message toast shows before it fades."
  :type 'number)

(defcustom romacs-layer-toast-max 3
  "Most message toasts shown at once; the oldest goes first."
  :type 'natnum)

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

(defun romacs-layer--toast-stack ()
  "Place the shown toasts at the top right, newest on top."
  (let* ((root (romacs-layer--root-frame))
         (cw (frame-char-width root)) (ch (frame-char-height root))
         (margin 12) (gap 8) (top margin))
    (dolist (toast romacs-layer--toasts)
      (let* ((frame (romacs-layer--frame (car toast)))
             (border (or (frame-parameter frame 'child-frame-border-width) 0))
             (size (romacs-layer--toast-size (nth 1 toast) (max 20 (/ (frame-text-cols root) 3))))
             ;; One column more: a window's last holds its continuation glyph.
             (width (* (1+ (car size)) cw)) (height (* (cdr size) ch)))
        (set-frame-size frame width height t)
        (set-frame-position frame (- (frame-text-width root) width (* 2 border) margin) top)
        (set-frame-parameter frame 'romacs-layer-edge 'top)
        ;; A new toast shows above every other layer, the panel too.
        (unless (frame-visible-p frame) (make-frame-visible frame) (raise-frame frame))
        (setq top (+ top height (* 2 border) gap))))))

(defun romacs-layer--toast-expire (buffer)
  "Hide the toast showing BUFFER and close up the stack."
  (setq romacs-layer--toasts (seq-remove (lambda (toast) (eq (car toast) buffer)) romacs-layer--toasts))
  (romacs-layer-hide buffer)
  (romacs-layer--toast-stack))

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
        (modify-frame-parameters frame (romacs-layer-glass-parameters romacs-layer-glass))
        (set-window-start (frame-root-window frame) 1))
      (push (list buffer text (run-at-time romacs-layer-toast-timeout nil #'romacs-layer--toast-expire buffer))
            romacs-layer--toasts)
      (romacs-layer--toast-stack))))

(provide 'romacs-layer)
;;; romacs-layer.el ends here
