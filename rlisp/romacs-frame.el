;;; romacs-frame.el --- One display model for frame.el and faces.el -*- lexical-binding: t; -*-
;; Copyright (C) 1993-1994, 1996-1997, 2000-2026 Free Software
;; Foundation, Inc.
;; Copyright (C) 1992-2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

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

(defun romacs--color-rgb (color frame)
  "The (RED GREEN BLUE) of COLOR, a list of them or a name, for `color-distance'."
  (or (if (consp color)
          (and (proper-list-p color) (= (length color) 3)
               (fixnump (nth 0 color)) (fixnump (nth 1 color)) (fixnump (nth 2 color))
               color)
        (and (stringp color) (color-values color frame)))
      (signal 'error (cons "Invalid color" (if (proper-list-p color) color (list color))))))

(defun color-distance (color1 color2 &optional frame metric)
  "Return an integer distance between COLOR1 and COLOR2 on FRAME.
COLOR1 and COLOR2 may be either strings containing the color name,
or lists of the form (RED GREEN BLUE), each in the range 0 to 65535
inclusive.  If FRAME is unspecified or nil, the current frame is used.
If METRIC is specified, it should be a function that accepts two
lists of the form (RED GREEN BLUE) aforementioned."
  (when (and frame (not (frame-live-p frame)))
    (signal 'wrong-type-argument (list 'frame-live-p frame)))
  (let ((rgb1 (romacs--color-rgb color1 frame))
        (rgb2 (romacs--color-rgb color2 frame)))
    (if metric
        (funcall metric rgb1 rgb2)
      (let ((mean (ash (+ (nth 0 rgb1) (nth 0 rgb2)) -1))
            (r (- (nth 0 rgb1) (nth 0 rgb2)))
            (g (- (nth 1 rgb1) (nth 1 rgb2)))
            (b (- (nth 2 rgb1) (nth 2 rgb2))))
        (ash (+ (ash (* (+ (* 2 65536) mean) r r) -16)
                (* 4 g g)
                (ash (* (- (+ (* 2 65536) 65535) mean) b b) -16))
             -16)))))

(defun color-supported-p (color &optional frame background-p)
  "Return non-nil if COLOR can be displayed on FRAME.
BACKGROUND-P non-nil means COLOR is used as a background.
COLOR must be a valid color name."
  (unless (stringp color) (signal 'wrong-type-argument (list 'stringp color)))
  (cond ((not (display-graphic-p frame)) (color-defined-p color frame))
        ((display-color-p frame) t)
        ((member (downcase color) '("black" "white")) t)
        ((or background-p (display-grayscale-p frame))
         (let ((rgb (color-values color frame)))
           (and rgb (= (nth 0 rgb) (nth 1 rgb) (nth 2 rgb)))))))

;;;; xfaces.c: `face-attributes-as-vector', merge_face_ref over a Lisp vector.

(defconst romacs--face-attribute-slots
  '((:family . 1) (:foundry . 2) (:width . 3) (:height . 4) (:weight . 5) (:slant . 6)
    (:underline . 7) (:inverse-video . 8) (:foreground . 9) (:background . 10)
    (:stipple . 11) (:overline . 12) (:strike-through . 13) (:box . 14) (:font . 15)
    (:distant-foreground . 18) (:extend . 19))
  "Each attribute `merge_face_ref' sets, and its slot in a Lisp face vector.")

(defun romacs--merge-face-height (from to invalid)
  "merge_face_heights: FROM, a height, over the height TO; INVALID if none."
  (cond ((integerp from) from)
        ((floatp from)
         (cond ((integerp to) (truncate (* from to)))
               ((floatp to) (* from to))
               ((eq to 'unspecified) from)
               (t invalid)))
        ((functionp from)
         (let ((height (condition-case nil (funcall from to) (error nil))))
           (if (or (integerp height) (floatp height)) height invalid)))
        (t invalid)))

(defun romacs--merge-face-attribute (keyword value to)
  "Merge face attribute KEYWORD's VALUE into the vector TO, if it is valid."
  (let ((valid
         (pcase keyword
           ((or :family :foundry :foreground :background :distant-foreground) (stringp value))
           ((or :weight :slant :width)
            (romacs--font-style-p (intern (substring (symbol-name keyword) 1)) value))
           (:underline (or (memq value '(t nil)) (stringp value) (consp value)))
           ((or :overline :strike-through) (or (memq value '(t nil)) (stringp value)))
           (:box (or (eq value t) (null value) (stringp value) (consp value)
                     (and (integerp value) (/= value 0))))
           ((or :inverse-video :extend) (memq value '(t nil)))
           (:stipple (or (null value) (bitmap-spec-p value)))
           (:font (fontp value)))))
    (when valid
      (aset to (alist-get keyword romacs--face-attribute-slots)
            (if (and (eq keyword :box) (eq value t)) 1 value)))))

(defun romacs--merge-face-vector (from to merging)
  "merge_face_vectors: the Lisp face vector FROM into TO."
  (let ((inherit (aref from 16)))
    (unless (memq inherit '(nil unspecified))
      (romacs--merge-face-ref inherit to merging)))
  (dotimes (i (length from))
    (let ((value (aref from i)))
      (cond ((or (= i 0) (= i 15) (eq value 'unspecified)))
            ((and (= i 4) (not (integerp value)))
             (aset to i (romacs--merge-face-height value (aref to i) (aref to i))))
            (t (aset to i value)))))
  (aset to 16 nil))

(defun romacs--merge-point-open-p (kind face merging)
  "push_named_merge_point's test: FACE is open as KIND in MERGING, newest first.
A remapping hides the plain merges of its face before it."
  (catch 'open
    (dolist (point merging)
      (when (eq (cdr point) face)
        (cond ((eq (car point) kind) (throw 'open t))
              ((eq (car point) 'remap) (throw 'open nil)))))
    nil))

(defun romacs--merge-named-face (face to merging)
  "merge_named_face: FACE's attributes into TO.
MERGING lists the merge points open, as (normal . FACE) and (remap . FACE):
FACE named inside its own remapping means the plain face."
  (let ((face (if (stringp face) (intern face) face)))
    (unless (romacs--merge-point-open-p 'normal face merging)
      (let* ((merging (cons (cons 'normal face) merging))
             (remapping (and (not (romacs--merge-point-open-p 'remap face merging))
                             (assq face face-remapping-alist))))
        (if remapping
            (let ((from (make-vector 20 'unspecified)))
              (romacs--merge-face-ref (cdr remapping) from (cons (cons 'remap face) merging))
              (romacs--merge-face-vector from to merging))
          (let ((from (and (symbolp face) (internal-lisp-face-p face (selected-frame)))))
            (when from (romacs--merge-face-vector from to merging))))))))

(defun romacs--merge-face-ref (ref to merging)
  "merge_face_ref: the face reference REF into the vector TO."
  (cond
   ((null ref))
   ((consp ref)
    (let ((first (car ref)))
      (cond
       ((eq first 'foreground-color) (when (stringp (cdr ref)) (aset to 9 (cdr ref))))
       ((eq first 'background-color) (when (stringp (cdr ref)) (aset to 10 (cdr ref))))
       ((eq first :filtered))
       ((keywordp first)
        (while (and (consp ref) (consp (cdr ref)))
          (let ((keyword (car ref)) (value (cadr ref)))
            (cond ((eq value 'unspecified))
                  ((eq keyword :height)
                   (let ((height (romacs--merge-face-height value (aref to 4) nil)))
                     (when height (aset to 4 height))))
                  ((eq keyword :inherit) (romacs--merge-face-ref value to merging))
                  ((assq keyword romacs--face-attribute-slots)
                   (romacs--merge-face-attribute keyword value to))))
          (setq ref (cddr ref))))
       (t
        (when (cdr ref) (romacs--merge-face-ref (cdr ref) to merging))
        (romacs--merge-face-ref first to merging)))))
   (t (romacs--merge-named-face ref to merging))))

(defun face-attributes-as-vector (plist)
  "Return a vector of face attributes corresponding to PLIST."
  (let ((to (make-vector 20 'unspecified)))
    (romacs--merge-face-ref plist to nil)
    to))

;;;; terminal.c, keyboard.c.

(defun delete-terminal (&optional terminal force)
  "Delete TERMINAL by deleting all frames on it and closing the terminal.
TERMINAL may be a terminal object, a frame, or nil (meaning the
selected frame's terminal).  Romacs has one terminal: it is the sole
active one, so even with FORCE it stays (VM2-WIN5)."
  (ignore force)
  (when (or (null terminal) (framep terminal) (terminal-live-p terminal))
    (error "Attempt to delete the sole active display terminal")))

(defun suspend-emacs (&optional stuffstring)
  "Stop Emacs and return to the superior process.
Run `suspend-hook' first and `suspend-resume-hook' after the process
continues.  STUFFSTRING has nowhere to go: Romacs has no terminal input."
  (interactive)
  (when stuffstring (unless (stringp stuffstring) (signal 'wrong-type-argument (list 'stringp stuffstring))))
  (run-hooks 'suspend-hook)
  (signal-process 0 'SIGTSTP)
  (run-hooks 'suspend-resume-hook)
  nil)

;;;; fontset.c over the leaves in prims/fontset.rs.

(defvar fontset-alias-alist
  (list (cons "-*-*-*-*-*-*-*-*-*-*-*-*-fontset-default" "fontset-default"))
  "Alist of fontset names vs the aliases.")

(defvar use-default-font-for-symbols t
  "If non-nil, use the default face's font for symbols and punctuation.
By default, Emacs will try to use the default face's font for
displaying symbol and punctuation characters, disregarding the
fontsets, if the default font can display the character.
Set this to nil to make Emacs honor the fontsets instead.")

(put 'fontset-info 'char-table-extra-slots 1)

(defun set-fontset-font (fontset characters font-spec &optional frame add)
  "Modify FONTSET to use font specification in FONT-SPEC for displaying CHARACTERS.

FONTSET should be a fontset name (a string); or nil, meaning the
fontset of FRAME; or t, meaning the default fontset.

CHARACTERS may be a single character, a cons (FROM . TO), a script
symbol, a charset symbol, or nil for any character for which no
font-spec is specified in FONTSET.

FONT-SPEC may be a font-spec object, a cons (FAMILY . REGISTRY), a
font name string, or nil, which explicitly specifies that there's no
font for CHARACTERS.

ADD, if non-nil, specifies how to add FONT-SPEC to the previously set
font specifications for CHARACTERS: `prepend' or `append'.  By default,
FONT-SPEC overwrites the previous settings."
  (when (romacs--set-fontset-font fontset characters font-spec frame add)
    (romacs--set-default-font font-spec frame))
  nil)

(defun romacs--set-default-font (font-spec frame)
  "Make FONT-SPEC, as `set-fontset-font' takes it, FRAME's ASCII font.
The default face is what paints ASCII, so its attributes change, each one
FONT-SPEC names; the frame keeps its fontset."
  (let* ((spec (cond ((consp font-spec) (font-spec :family (car font-spec)))
                     ((stringp font-spec) (font-spec :name font-spec))
                     (t font-spec)))
         (attributes
          (mapcan (lambda (key)
                    (let ((value (font-get spec key)))
                      (when value
                        (list key (if (eq key :family) (symbol-name value) value)))))
                  '(:family :weight :slant :width))))
    (when attributes
      (apply #'set-face-attribute 'default (or frame (selected-frame)) attributes))))

(defun romacs--fontset-info-table (table runs)
  "Fill the char-table TABLE from RUNS, (FALLBACK (FROM TO . ALIST) ...)."
  (when (car runs) (set-char-table-range table nil (car runs)))
  (dolist (run (cdr runs))
    (set-char-table-range table (cons (car run) (cadr run)) (cddr run))))

(defun fontset-info (fontset &optional frame)
  "Return information about a fontset FONTSET on frame FRAME.

FONTSET is a fontset name string, nil for the fontset of FRAME, or t
for the default fontset.  FRAME nil means the selected frame.

The value is a char-table whose elements have this form:

    ((FONT OPENED-FONT ...) ...)

FONT is a name of font specified for a range of characters.

OPENED-FONT is a name of a font actually opened.

The char-table has one extra slot.  If FONTSET is not the default
fontset, the value the extra slot is a char-table containing the
information about the derived fonts from the default fontset.  The
format is the same as above."
  (let ((runs (romacs--fontset-info fontset frame))
        (table (make-char-table 'fontset-info)))
    (romacs--fontset-info-table table (car runs))
    (when (cdr runs)
      (let ((shared (make-char-table nil)))
        (romacs--fontset-info-table shared (cdr runs))
        (set-char-table-extra-slot table 0 shared)))
    table))

(defun x-list-fonts (pattern &optional _face frame maximum _width)
  "Return a list of the names of available fonts matching PATTERN.
If optional arguments FACE and FRAME are specified, return only fonts
the same size as FACE on FRAME.  MAXIMUM limits the number of names."
  (mapcar (lambda (font) (font-xlfd-name font))
          (list-fonts (font-spec :name pattern) frame maximum)))

(defun romacs--make-default-fontsets ()
  "The default fontset's rules and the standard fontset."
  (create-default-fontset)
  (create-fontset-from-fontset-spec standard-fontset-spec t))

(defun romacs--setup-fontsets ()
  "What a window system's initialization does for fontsets: the default
fontset's rules, the standard fontset, and the frame's own fontset from its
font (fontset_from_font)."
  (unless (romacs--unstage-fontsets)
    (romacs--make-default-fontsets))
  (let* ((font (frame-parameter nil 'font))
         (spec (font-spec :name font))
         (registry (font-get spec :registry))
         (target (let ((encoding (romacs--font-encoding (symbol-name registry))))
                   (let ((charset (if (consp encoding) (cdr encoding) encoding)))
                     (if (charsetp charset) charset 'latin))))
         (rule (font-spec :registry registry)))
    (font-put spec :registry 'fontset-startup)
    (let ((name (font-xlfd-name spec nil t)))
      (romacs--make-frame-fontset name (list "fontset-startup" (downcase font)) font)
      (romacs--set-fontset-font name target rule)
      (romacs--set-fontset-font name nil rule))))

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

;; A child frame shows its parent's faces until something sets one on it
;; alone (`romacs--frame-shares-faces-p', DIVERGENCES.md VM2-CHILD-FRAME):
;; recomputing a face there would only repeat the parent's, so
;; `face-spec-recalc' skips it, and a theme or a `defface' costs one
;; frame's work however many layers are open.
(defun romacs--face-spec-recalc-shared (_face frame)
  "Non-nil when FRAME shows another frame's faces, which `face-spec-recalc'
already recomputes there."
  (romacs--frame-shares-faces-p frame))
(advice-add 'face-spec-recalc :before-until #'romacs--face-spec-recalc-shared)

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
