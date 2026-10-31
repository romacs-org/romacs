;;; romacs-pgtk.el --- GNU pgtk's C entry points on Romacs's display  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; A frame on Romacs's GUI display is a `pgtk' frame (VM2-WIN5), so the
;; backend methods term/pgtk-win.el defines for that window system run,
;; and frame.el and browse-url.el dispatch to pgtk's C primitives.  These
;; are those primitives over Romacs's display (VM2-WINSYS1): this session's
;; own selections are kept here and converted through
;; `selection-converter-alist' as GNU converts them; CLIPBOARD is also the
;; desktop's, through Romacs's clipboard, and another program copying ends
;; this session's ownership as GDK's selection-clear does; a foreign
;; PRIMARY or SECONDARY is not read.  Romacs's window reads text through
;; its own input method, so there is no GtkIMContext to switch.  The frame
;; is one undecorated Wayland surface: no title bar, external menu or tool
;; bar, no border, and no position on the display, as pinned GNU pgtk
;; reports a Wayland frame's.  `make-frame' makes child frames, drawn over
;; it (VM2-CHILD-FRAME).

;;; Code:

(defvar pgtk-use-im-context-on-new-connection t
  "Whether to use GtkIMContext on a new connection.
If you want to change it after connection, use the `pgtk-use-im-context'
function.")

(defvar romacs-pgtk--selections nil
  "Alist of (SELECTION . VALUE) for each selection this session owns.")

(defvar romacs-pgtk--clipboard nil
  "The text this session last put on the desktop's CLIPBOARD.")

(defun romacs-pgtk--owned (selection)
  "SELECTION's (SELECTION . VALUE) entry while this session owns it.
Once the desktop's CLIPBOARD holds anything but the text this session
put there (other text, no text, or something not text), another
program owns it, and this session's entry goes; a clipboard that cannot
be read changes nothing."
  (let ((entry (assq selection romacs-pgtk--selections)))
    (when (and entry (eq selection 'CLIPBOARD) romacs-pgtk--clipboard)
      (let ((desktop (condition-case nil (romacs--clipboard-text) (error 'unread))))
        (unless (or (eq desktop 'unread) (equal desktop romacs-pgtk--clipboard))
          (setq romacs-pgtk--selections (assq-delete-all 'CLIPBOARD romacs-pgtk--selections)
                romacs-pgtk--clipboard nil
                entry nil))))
    entry))

(defun romacs-pgtk--frame (frame)
  "Signal unless FRAME (nil for the selected one) is a live pgtk frame."
  (unless (eq (frame-live-p (or frame (selected-frame))) 'pgtk)
    (error "GDK selection unavailable for this frame")))

(defun romacs-pgtk--name (selection)
  "SELECTION as pgtkselect.c reads it: nil is PRIMARY and t SECONDARY."
  (unless (symbolp selection) (signal 'wrong-type-argument (list 'symbolp selection)))
  (cond ((null selection) 'PRIMARY) ((eq selection t) 'SECONDARY) (t selection)))

(defun romacs-pgtk--local (selection target)
  "This session's SELECTION as TARGET, converted as pgtk_get_local_selection
does for a request from this session: the converter sees no target, so
text stays text."
  (let* ((value (cdr (assq selection romacs-pgtk--selections)))
         (handler (cdr (assq target selection-converter-alist)))
         (handler (if (consp handler) (cdr handler) handler))
         (value (or (and (stringp value) (get-text-property 0 target value)) value))
         (converted (and value handler (funcall handler selection nil value))))
    ;; A (TYPE . DATA) answer is DATA, a one-element list its element.
    (when (and (consp converted) (symbolp (car converted)))
      (setq converted (cdr converted))
      (when (and (consp converted) (null (cdr converted)))
        (setq converted (car converted))))
    converted))

(defun pgtk-own-selection-internal (selection value &optional frame)
  "Assert a selection of type SELECTION and value VALUE.
SELECTION is a symbol, typically `PRIMARY', `SECONDARY', or `CLIPBOARD'.
VALUE is typically a string, or a cons of two markers, but may be
anything that the functions on `selection-converter-alist' know about.
FRAME should be a frame that should own the selection."
  (romacs-pgtk--frame frame)
  (unless (symbolp selection) (signal 'wrong-type-argument (list 'symbolp selection)))
  (unless value (error "VALUE may not be nil"))
  (setf (alist-get selection romacs-pgtk--selections) value)
  (when (eq selection 'CLIPBOARD)
    (let ((text (romacs-pgtk--local 'CLIPBOARD 'UTF8_STRING)))
      (when (stringp text)
        (setq text (substring-no-properties text)
              romacs-pgtk--clipboard text)
        (if (fboundp 'romacs-clipboard-select-text)
            (romacs-clipboard-select-text text)
          (romacs--clipboard-set text)))))
  value)

(defun pgtk-disown-selection-internal (selection &optional _time-object _terminal)
  "If we own the selection SELECTION, disown it."
  (unless (symbolp selection) (signal 'wrong-type-argument (list 'symbolp selection)))
  (when (romacs-pgtk--owned selection)
    (setq romacs-pgtk--selections (assq-delete-all selection romacs-pgtk--selections))
    t))

(defun pgtk-selection-owner-p (&optional selection _terminal)
  "Whether the current Emacs process owns the given selection."
  (and (romacs-pgtk--owned (romacs-pgtk--name selection)) t))

(defun pgtk-selection-exists-p (&optional selection _terminal)
  "Whether there is an owner for the given selection."
  (let ((selection (romacs-pgtk--name selection)))
    (or (and (romacs-pgtk--owned selection) t)
        (and (eq selection 'CLIPBOARD)
             (stringp (ignore-errors (romacs--clipboard-text)))))))

(defun pgtk-get-selection-internal (selection-symbol target-type &optional _time-stamp _terminal)
  "Return text selected from some window.
SELECTION-SYMBOL is typically `PRIMARY', `SECONDARY', or `CLIPBOARD'.
TARGET-TYPE is the type of data desired, typically `STRING'."
  (unless (symbolp selection-symbol) (signal 'wrong-type-argument (list 'symbolp selection-symbol)))
  (unless (symbolp target-type) (signal 'wrong-type-argument (list 'symbolp target-type)))
  (when (eq target-type 'MULTIPLE)
    (error "Retrieving MULTIPLE selections is currently unimplemented"))
  (romacs-pgtk--frame nil)
  (if (romacs-pgtk--owned selection-symbol)
      (romacs-pgtk--local selection-symbol target-type)
    ;; The desktop's CLIPBOARD, as text; TARGETS a vector, as pgtkselect.c
    ;; answers it for a foreign owner.
    (let ((text (and (eq selection-symbol 'CLIPBOARD) (ignore-errors (romacs--clipboard-text)))))
      (cond ((eq target-type 'TARGETS)
             (if (stringp text) [UTF8_STRING STRING TEXT] []))
            ((memq target-type '(UTF8_STRING STRING TEXT COMPOUND_TEXT text/plain\;charset=utf-8))
             text)))))

(defun pgtk-use-im-context (_use-p &optional terminal)
  "Set whether to use GtkIMContext.
Romacs's window reads text through its own input method, which stays in use."
  (unless (terminal-live-p terminal)
    (signal 'wrong-type-argument (list 'terminal-live-p terminal)))
  nil)

;; frame.el's frame creation (`make-frame'): Romacs's display makes
;; child frames, painted over their parent (VM2-CHILD-FRAME).  Faces are
;; the session's, not a frame's, so `x-create-frame-with-faces' would have
;; no frame faces to set up.

(cl-defmethod frame-creation-function (params &context (window-system pgtk))
  (romacs--create-frame params))

(defun x-get-resource (_attribute _class &optional _component _subclass)
  "Return the value of ATTRIBUTE, of class CLASS, from the X defaults database.
Romacs's display has no resource database, so there is none."
  nil)

;; frame.el's geometry dispatch (`frame-geometry', `frame-edges',
;; `mouse-absolute-pixel-position', `frame-restack').

(defun romacs-pgtk--border (frame)
  "FRAME's internal border in pixels: a child frame's border, else 0."
  (if (frame-parent frame) (/ (- (frame-native-width frame) (frame-text-width frame)) 2) 0))

(defun pgtk-frame-geometry (&optional frame)
  "Return geometric attributes of FRAME, an alist; sizes are in pixels.
Romacs's frame has no title bar, external menu or tool bar, tab bar or
border, and a Wayland surface has no position on its display; a child
frame's position is in its parent."
  (let ((size (cons (frame-native-width frame) (frame-native-height frame))))
    `((outer-position . ,(frame-position frame)) (outer-size . ,size) (external-border-size 0 . 0)
      (title-bar-size 0 . 0) (menu-bar-external) (menu-bar-size 0 . 0)
      (tab-bar-size 0 . 0) (tool-bar-external) (tool-bar-position . top)
      (tool-bar-size 0 . 0) (internal-border-width . ,(romacs-pgtk--border frame)))))

(defun pgtk-frame-edges (&optional frame type)
  "Return edge coordinates of FRAME, (LEFT TOP RIGHT BOTTOM) in pixels.
Romacs's frame has no decorations, so its outer and native edges are the
same: a child frame's in its parent, the root frame's at the origin.
TYPE `inner-edges' excludes a child frame's internal border."
  (let* ((position (frame-position frame))
         (left (car position)) (top (cdr position))
         (inset (if (eq type 'inner-edges) (romacs-pgtk--border frame) 0)))
    (list (+ left inset) (+ top inset)
          (- (+ left (frame-native-width frame)) inset)
          (- (+ top (frame-native-height frame)) inset))))

(defun pgtk-mouse-absolute-pixel-position ()
  "Return absolute position of mouse cursor in pixels, (0 . 0) on Wayland."
  (cons 0 0))

(defun pgtk-set-mouse-absolute-pixel-position (_x _y)
  "Move mouse pointer to absolute pixel position; Wayland moves no pointer."
  nil)

(defun pgtk-frame-restack (frame1 frame2 &optional above)
  "Restack FRAME1 below FRAME2, or above it with ABOVE.
Child frames stack among themselves; the root frame stays where it is."
  (unless (frame-live-p frame1) (signal 'wrong-type-argument (list 'frame-live-p frame1)))
  (unless (frame-live-p frame2) (signal 'wrong-type-argument (list 'frame-live-p frame2)))
  (when (frame-parent frame1)
    (if above (raise-frame frame1) (lower-frame frame1)))
  t)

;; browse-url.el's dispatch for a pgtk frame.

(defun pgtk-backend-display-class (&optional _terminal)
  "Return the name of the GDK backend display class of TERMINAL."
  (interactive "")
  (if (getenv "WAYLAND_DISPLAY") "GdkWaylandDisplay" "GdkX11Display"))

(defun x-gtk-launch-uri (frame uri)
  "Launch the desktop's default application to show URI, as GTK would.
Romacs asks `xdg-open', and signals as GNU does when it fails."
  (unless (framep frame) (signal 'wrong-type-argument (list 'framep frame)))
  (unless (eq (frame-live-p frame) 'pgtk)
    (error "GTK URI launch not available for this frame"))
  (unless (stringp uri) (signal 'wrong-type-argument (list 'stringp uri)))
  (let ((status (condition-case err (call-process "xdg-open" nil nil nil uri)
                  (error (error-message-string err)))))
    (unless (eql status 0)
      (error "Failed to launch URI via GTK: %s"
             (if (stringp status) status (format "xdg-open exited with %s" status)))))
  nil)

(provide 'romacs-pgtk)
;;; romacs-pgtk.el ends here
