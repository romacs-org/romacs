;;; romacs-layer.el --- Buffers shown in a layer over the frame  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; A layer is a child frame (GNU's `parent-frame') showing one buffer over
;; the root frame.  Showing, moving or hiding it changes no window of the
;; frame below and no terminal's size (DIVERGENCES.md VM2-CHILD-FRAME).
;; Where a layer goes is policy, here: a full-width strip directly above
;; the minibuffer line (or the minibuffer's own layer) by default, a centred box, or below point.  A
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
  "Where a `bottom' layer of ROOT ends: above the minibuffer line, or above
the minibuffer's layer when a child frame hosts the minibuffer window
\(`romacs--host-minibuffer'), else at ROOT's bottom edge."
  (let ((host (romacs--minibuffer-host)))
    (cond ((null host) (window-pixel-top (minibuffer-window root)))
          ((frame-visible-p host) (cdr (frame-position host)))
          (t (frame-text-height root)))))

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
      (set-frame-position frame left top))
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

(provide 'romacs-layer)
;;; romacs-layer.el ends here
