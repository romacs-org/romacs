;;; romacs-frame.el --- One display model for frame.el and faces.el -*- lexical-binding: t; -*-
;; Copyright (C) 1993-1994, 1996-1997, 2000-2026 Free Software
;; Foundation, Inc.
;; Copyright (C) 1992-2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; GNU's frame.el and faces.el answer display questions by dispatching on
;; the frame type to x-, w32-, ns-, pgtk-, haiku-, android- and tty- C
;; primitives.  Romacs has one display, described by one capability
;; record, `romacs-display'; the functions below read it instead
;; (DIVERGENCES.md VM2-DISPLAY1).  Loaded after frame.el, faces.el and
;; term/tty-colors.el.
;;
;; Without a GUI the record is `romacs-display-headless': what pinned GNU
;; reports in batch for its initial terminal, so package tests see GNU's
;; batch answers.
;;
;; Colors keep tty-colors.el's model (names, specs, approximation), fed
;; from the record: a display with a :palette approximates to it, and one
;; with 16777216 color cells and no palette shows every color exactly
;; (`tty-color-24bit').
;;
;; Adapted from GNU frame.el: `frame-set-background-mode' is GNU's, with
;; the display type read from the record.

;;; Code:

(defconst romacs-display-headless
  `(:graphic nil :mouse nil :selections nil :symbol-keys nil
    :screens 1 :pixel-width 80 :pixel-height 25 :mm-width nil :mm-height nil
    :backing-store not-useful :save-under not-useful
    :planes 3 :color-cells 0 :visual-class static-gray
    :color nil :grayscale nil :display-type mono :background-mode dark
    :palette ,tty-standard-colors)
  "The capabilities of the display Romacs has without a GUI.
Each value is pinned GNU's batch answer for its initial terminal.")

(defvar romacs-display romacs-display-headless
  "The capabilities of Romacs's display, a plist.
Keys: :graphic, :mouse, :selections, :symbol-keys (booleans); :screens,
:pixel-width, :pixel-height, :mm-width, :mm-height, :planes,
:color-cells (numbers, mm sizes nil when unknown); :backing-store and
:save-under (symbols); :visual-class, :display-type (`color', `grayscale'
or `mono') and :background-mode (`light' or `dark'); :color, :grayscale
(booleans); :palette, the (NAME INDEX R G B) colors the display can show,
or nil when it shows any color.  The display functions of frame.el and
faces.el read this record; the GUI sets it when it starts.")

(defun romacs-display-get (key)
  "Return the KEY capability of Romacs's display."
  (plist-get romacs-display key))

;;;; frame.el: display queries.  DISPLAY names Romacs's one display.

(defun display-mouse-p (&optional _display)
  "Return non-nil if DISPLAY has a mouse available."
  (romacs-display-get :mouse))

(defun display-graphic-p (&optional _display)
  "Return non-nil if DISPLAY is a graphic display."
  (romacs-display-get :graphic))

(defun display-selections-p (&optional _display)
  "Return non-nil if DISPLAY supports selections."
  (romacs-display-get :selections))

(defun display-symbol-keys-p (&optional _display)
  "Return non-nil if DISPLAY supports symbol names as keys."
  (romacs-display-get :symbol-keys))

(defun display-screens (&optional _display)
  "Return the number of screens associated with DISPLAY."
  (romacs-display-get :screens))

(defun display-pixel-height (&optional _display)
  "Return the height of DISPLAY's screen in pixels."
  (romacs-display-get :pixel-height))

(defun display-pixel-width (&optional _display)
  "Return the width of DISPLAY's screen in pixels."
  (romacs-display-get :pixel-width))

(defun romacs--display-mm (display select key)
  "DISPLAY's size in millimeters: SELECT of `display-mm-dimensions-alist',
else the record's KEY, on a graphic display only."
  (and (romacs-display-get :graphic)
       (or (funcall select (cdr (assoc (or display (frame-parameter nil 'display))
                                       display-mm-dimensions-alist)))
           (funcall select (cdr (assoc t display-mm-dimensions-alist)))
           (romacs-display-get key))))

(defun display-mm-height (&optional display)
  "Return the height of DISPLAY's screen in millimeters, or nil."
  (romacs--display-mm display #'cdr :mm-height))

(defun display-mm-width (&optional display)
  "Return the width of DISPLAY's screen in millimeters, or nil."
  (romacs--display-mm display #'car :mm-width))

(defun display-backing-store (&optional _display)
  "Return the backing store capability of DISPLAY's screen."
  (romacs-display-get :backing-store))

(defun display-save-under (&optional _display)
  "Return non-nil if DISPLAY's screen supports the SaveUnder feature."
  (romacs-display-get :save-under))

(defun display-planes (&optional _display)
  "Return the number of planes supported by DISPLAY."
  (romacs-display-get :planes))

(defun face-font (face &optional frame _character)
  "Return the font name of face FACE, or nil if it is unspecified.
The font name is, by default, for ASCII characters.
If the optional argument FRAME is given, report on face FACE in that frame.
If FRAME is t, report on the defaults for face FACE (for new frames).
  The font default for a face is either nil, or a list
  of the form (bold), (italic) or (bold italic).
If FRAME is omitted or nil, use the selected frame.
xfaces.c Fface_font: Romacs's frame is a terminal frame, which realizes no
font, so a frame's answer is nil once FACE is known."
  ;; resolve_face_name: a string names the face of that symbol.
  (when (stringp face) (setq face (intern face)))
  (if (eq frame t)
      (let ((result nil)
            (weight (face-attribute face :weight t))
            (slant (face-attribute face :slant t)))
        (unless (memq weight '(unspecified normal)) (push 'bold result))
        (unless (memq slant '(unspecified normal)) (push 'italic result))
        result)
    (unless (frame-live-p (or frame (selected-frame)))
      (signal 'wrong-type-argument (list 'frame-live-p frame)))
    (unless (facep face) (signal 'error (list "Invalid face" face)))
    nil))

(defun display-color-cells (&optional _display)
  "Return the number of color cells supported by DISPLAY."
  (romacs-display-get :color-cells))

(defun display-visual-class (&optional _display)
  "Return the visual class of DISPLAY."
  (romacs-display-get :visual-class))

(defun display-monitor-attributes-list (&optional display)
  "Return a list of physical monitor attributes on DISPLAY: one monitor."
  (let ((geometry (list 0 0 (display-pixel-width display)
                        (display-pixel-height display))))
    `(((geometry . ,geometry)
       (workarea . ,geometry)
       (mm-size . (,(display-mm-width display) ,(display-mm-height display)))
       (frames . ,(frames-on-display-list display))))))

;;;; faces.el: colors.

(defun display-color-p (&optional _display)
  "Return t if DISPLAY supports color."
  (romacs-display-get :color))

(defun display-grayscale-p (&optional _display)
  "Return non-nil if frames on DISPLAY can display shades of gray."
  (romacs-display-get :grayscale))

(defun tty-color-alist (&optional _frame)
  "Return the colors Romacs's display can show, as (NAME INDEX R G B).
Nil when it shows any color."
  (romacs-display-get :palette))

(defun defined-colors (&optional _frame)
  "Return a list of colors supported for a particular frame."
  (mapcar #'car (or (romacs-display-get :palette) color-name-rgb-alist)))

(defun color-defined-p (color &optional frame)
  "Return non-nil if COLOR is supported on frame FRAME."
  (unless (member color '(unspecified "unspecified-bg" "unspecified-fg"))
    (numberp (tty-color-translate color frame))))

(defun color-values (color &optional frame)
  "Return a description of the color named COLOR on frame FRAME.
The value is (RED GREEN BLUE), each between 0 and 65535, or nil."
  (unless (member color '(unspecified "unspecified-fg" "unspecified-bg"))
    (tty-color-values color frame)))

;;;; frame.el: background mode.

(defun frame--current-background-mode (frame)
  (let ((default-bg-mode (romacs-display-get :background-mode))
        (bg-color (frame-parameter frame 'background-color)))
    (cond (frame-background-mode)
          ((equal bg-color "unspecified-fg") ; inverted colors
           (if (eq default-bg-mode 'light) 'dark 'light))
          ((not (color-values bg-color frame))
           default-bg-mode)
          ((color-dark-p (mapcar (lambda (c) (/ c 65535.0))
                                 (color-values bg-color frame)))
           'dark)
          (t 'light))))

(defun frame-set-background-mode (frame &optional keep-face-specs)
  "Set up display-dependent faces on FRAME.
Display-dependent faces are those which have different definitions
according to the `background-mode' and `display-type' frame parameters.

If optional arg KEEP-FACE-SPECS is non-nil, don't recalculate
face specs for the new background mode."
  (unless inhibit-frame-set-background-mode
    (let* ((bg-mode (frame--current-background-mode frame))
	   (display-type (romacs-display-get :display-type))
	   (old-bg-mode (frame-parameter frame 'background-mode))
	   (old-display-type (frame-parameter frame 'display-type)))
      (unless (and (eq bg-mode old-bg-mode) (eq display-type old-display-type))
	(let ((locally-modified-faces nil)
	      ;; Prevent face-spec-recalc from calling this function
	      ;; again, resulting in a loop (bug#911).
	      (inhibit-frame-set-background-mode t)
	      (params (list (cons 'background-mode bg-mode)
			    (cons 'display-type display-type))))
	  (if keep-face-specs
	      (modify-frame-parameters frame params)
	    ;; Faces that don't match their face-specs were modified on
	    ;; FRAME; leave them alone.  A face not yet realized for FRAME
	    ;; counts as mismatched, so face-spec-recalc realizes it.
	    (dolist (face (face-list))
	      (and (not (get face 'face-override-spec))
		   (not (and (gethash face (frame--face-hash-table))
                             (face-spec-match-p face
                                                (face-user-default-spec face)
                                                frame)))
		   (push face locally-modified-faces)))
	    (modify-frame-parameters frame params)
	    (dolist (face (face-list))
	      (unless (memq face locally-modified-faces)
		(face-spec-recalc face frame)))))))))

;; composite.c's `find-composition-internal'. VM2 builds no composition
;; glyph strings: where GNU would find a static composition, or an automatic
;; one could form (the current buffer shows in a window and a character near
;; POS has a `composition-function-table' entry), it refuses
;; (VM2-COMPOSE1). Everywhere else GNU's answer is nil.

(defun romacs--composition-position (pos)
  "GNU fix_position: a marker's position or an integer, else an error."
  (cond ((markerp pos)
         (or (marker-position pos) (error "Marker does not point anywhere")))
        ((integerp pos) pos)
        (t (signal 'wrong-type-argument (list 'integer-or-marker-p pos)))))

(defun find-composition-internal (pos limit string detail-p)
  "Internal use only.

Return information about composition at or nearest to position POS.
See `find-composition' for more details."
  (ignore detail-p)
  (let* ((from (romacs--composition-position pos))
         (to (if limit (min (romacs--composition-position limit) (point-max)) -1)))
    (if string
        (progn
          (unless (stringp string) (signal 'wrong-type-argument (list 'stringp string)))
          (unless (<= 0 from (length string)) (signal 'args-out-of-range (list string pos))))
      (unless (<= (point-min) from (point-max))
        (signal 'args-out-of-range (list (current-buffer) pos))))
    ;; find_composition probes the property outside any narrowing.
    (when (save-restriction
            (unless string (widen))
            (or (get-text-property from 'composition string)
                (and (>= to 0) (> to from)
                     (/= (next-single-property-change from 'composition string to) to))
                (and (>= to 0) (< to from)
                     (or (and (> from (if string 0 (point-min)))
                              (get-text-property (1- from) 'composition string))
                         (/= (previous-single-property-change from 'composition string to) to)))))
      (error "VM2 find-composition-internal on a static composition is unsupported"))
    (when (and (if string (multibyte-string-p string) enable-multibyte-characters)
               auto-composition-mode
               (get-buffer-window (current-buffer)))
      (let* ((limit (if (< to 0) from to))
             (head (if string 0 (save-excursion (goto-char from) (pos-bol))))
             (tail (if string (length string) (point-max)))
             (end (min tail (if (<= limit from) (+ from 1 3) (+ limit 3))))
             (i head))
        (while (< i end)
          (when (aref composition-function-table (if string (aref string i) (char-after i)))
            (error "VM2 find-composition-internal with automatic composition is unsupported"))
          (setq i (1+ i)))))
    nil))

(provide 'romacs-frame)
;;; romacs-frame.el ends here
