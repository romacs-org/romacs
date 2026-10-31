;;; romacs-keymap.el --- GNU's active-keymap policy in Lisp -*- lexical-binding: t; -*-
;; Copyright (C) 1985-1988, 1993-1995, 1998-2026 Free Software
;; Foundation, Inc.
;; Copyright (C) 1985-1989, 1993-1997, 1999-2026 Free Software Foundation,
;; Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Which keymaps are active is policy over Lisp-visible state: the
;; overriding maps, the emulation and minor-mode alists, the `keymap' and
;; `local-map' properties at point and the local and global maps.  GNU
;; computes it in C; Romacs computes it here, in GNU's order.
;;
;; `key-binding' is GNU's composition over them: the native primitive
;; consults only the local and global maps, so a bare VM keeps it until
;; this file replaces it.
;;
;; Adapted from GNU src/keymap.c (`current-active-maps', `key-binding',
;; `current_minor_maps', `click_position') and src/keyboard.c
;; (`posn-point', `menu_item_eval_property').

;;; Code:

(defun posn-point (position)
  "Return the buffer location in POSITION, or nil.
POSITION is a list of the form returned by `event-start' and `event-end'."
  (or (nth 5 position)
      (let ((posn (nth 1 position)))
        (cond ((consp posn) (car posn))
              ((integerp posn) posn)))))

(defun romacs--menu-filter (filter definition)
  "Apply a menu item's FILTER to its DEFINITION, as keymap lookup does.
Errors give nil (keyboard.c menu_item_eval_property)."
  (let ((inhibit-redisplay t))
    (condition-case nil (eval (list filter (list 'quote definition)) nil) (error nil))))

(defun romacs-keymap--get (object)
  "OBJECT as a keymap without autoloading, or nil (keymap.c get_keymap)."
  (cond ((and (consp object) (eq (car object) 'keymap)) object)
        ((and object (symbolp object))
         (let ((f (indirect-function object)))
           (and (consp f) (eq (car f) 'keymap) f)))))

(defun romacs-keymap--local-map (position type)
  "The keymap of property TYPE at POSITION (intervals.c get_local_map).
The character after POSITION wins, then the property POSITION itself
would give inserted text; narrowing is ignored.  For `local-map', the
buffer's local map is the fallback."
  (let* ((position (max (point-min) (min position (point-max))))
         (map (romacs-keymap--get
               (save-restriction
                 (widen)
                 (or (get-char-property position type)
                     (get-pos-property position type))))))
    (or map (and (eq type 'local-map) (current-local-map)))))

(defun romacs-keymap--minor-maps ()
  "The active minor-mode keymaps, highest priority first (current_minor_maps)."
  (mapcar #'cdr (romacs-keymap--minor-mode-maps)))

(defun romacs-keymap--minor-mode-maps ()
  "The active minor modes' (MODE . KEYMAP), highest priority first
\(current_minor_maps)."
  (let ((maps nil)
        (alists (append (mapcar (lambda (alist)
                                  (if (symbolp alist) (and (boundp alist) (symbol-value alist)) alist))
                                (if (consp emulation-mode-map-alists) emulation-mode-map-alists))
                        (list minor-mode-overriding-map-alist minor-mode-map-alist))))
    (dolist (alist alists)
      (dolist (entry alist)
        (when (and (consp entry) (symbolp (car entry)) (boundp (car entry))
                   (symbol-value (car entry))
                   ;; A minor-mode-map-alist entry is ignored when the mode
                   ;; is overridden.
                   (not (and (eq alist minor-mode-map-alist)
                             (assq (car entry) minor-mode-overriding-map-alist))))
          (let ((map (indirect-function (cdr entry))))
            (when map (push (cons (car entry) map) maps))))))
    (nreverse maps)))

(defun minor-mode-key-binding (key &optional accept-default)
  "Find the visible minor mode bindings of KEY.
Return an alist of pairs (MODENAME . BINDING), where MODENAME is
the symbol which names the minor mode binding KEY, and BINDING is
KEY's definition in that mode.  In particular, if KEY has no
minor-mode bindings, return nil.  If the first binding is a
non-prefix, all subsequent bindings will be omitted, since they would
be ignored.  Similarly, the list doesn't include non-prefix bindings
that come after prefix bindings.

If optional argument ACCEPT-DEFAULT is non-nil, recognize default
bindings; see the description of `lookup-key' for more details about this."
  (let ((prefixes nil))
    (catch 'found
      (dolist (entry (romacs-keymap--minor-mode-maps))
        (let ((binding (lookup-key (cdr entry) key accept-default)))
          (when (and binding (not (integerp binding)))
            (cond ((keymapp binding) (push (cons (car entry) binding) prefixes))
                  ((null prefixes) (throw 'found (list (cons (car entry) binding))))))))
      (nreverse prefixes))))

(defun current-active-maps (&optional olp position)
  "Return a list of the currently active keymaps.
OLP if non-nil indicates that we should obey `overriding-local-map' and
`overriding-terminal-local-map'.  POSITION can specify a click position
like in the respective argument of `key-binding' or a live window which
means to return the active maps for that window's buffer."
  (let ((window (cond ((consp position) (posn-window position))
                      ((window-live-p position) position))))
    (with-current-buffer (if (and (windowp window) (bufferp (window-buffer window)))
                             (window-buffer window)
                           (current-buffer))
      (let ((keymaps (list (current-global-map))))
        (when (and olp (null overriding-terminal-local-map) overriding-local-map)
          (push overriding-local-map keymaps))
        (when (null (cdr keymaps))
          (let* ((pt (cond ((or (integerp position) (markerp position))
                            (let ((pos (if (markerp position) (marker-position position) position)))
                              (unless (and pos (<= (point-min) pos (point-max)))
                                (signal 'args-out-of-range (list (current-buffer) position)))
                              pos))
                           (t (point))))
                 (local-map (romacs-keymap--local-map pt 'local-map))
                 (keymap (romacs-keymap--local-map pt 'keymap)))
            (when (consp position)
              ;; A click: the properties of the clicked string, then of the
              ;; clicked buffer position, then the buffer's local map.
              (setq local-map nil keymap nil)
              (let ((string (posn-string position)))
                (when (and (consp string) (stringp (car string))
                           (integerp (cdr string))
                           (<= 0 (cdr string)) (< (cdr string) (length (car string))))
                  (setq local-map (get-text-property (cdr string) 'local-map (car string))
                        keymap (get-text-property (cdr string) 'keymap (car string)))))
              (let ((buffer-posn (posn-point position)))
                (when (and (integerp buffer-posn)
                           (<= (point-min) buffer-posn) (<= buffer-posn (1+ (buffer-size))))
                  (unless local-map (setq local-map (romacs-keymap--local-map buffer-posn 'local-map)))
                  (unless keymap (setq keymap (romacs-keymap--local-map buffer-posn 'keymap)))))
              (unless local-map (setq local-map (current-local-map))))
            (when local-map (push local-map keymaps))
            (setq keymaps (append (romacs-keymap--minor-maps) keymaps))
            (when keymap (push keymap keymaps))
            (when (and olp overriding-terminal-local-map)
              (push overriding-terminal-local-map keymaps))))
        keymaps))))

(defun key-binding (key &optional accept-default no-remap position)
  "Return the binding for command KEY in current keymaps.
KEY is a string or vector, a sequence of keystrokes.  The binding is
probably a symbol with a function definition.

Normally, `key-binding' ignores bindings for t, which act as default
bindings, used when nothing else in the keymap applies; this makes it
usable as a general function for probing keymaps.  However, if the
optional second argument ACCEPT-DEFAULT is non-nil, `key-binding' does
recognize the default bindings, just as `read-key-sequence' does.

Like the normal command loop, `key-binding' will remap the command
resulting from looking up KEY by looking up the command in the
current keymaps.  However, if the optional third argument NO-REMAP
is non-nil, `key-binding' returns the unmapped command.

If KEY is a key sequence initiated with the mouse, the used keymaps
will depend on the clicked mouse position with regard to the buffer
and possible local keymaps on strings.

If the optional argument POSITION is non-nil, it specifies a mouse
position as returned by `event-start' and `event-end', and the lookup
occurs in the keymaps associated with it instead of KEY.  It can also
be a number or marker, in which case the keymap properties at the
specified buffer position instead of point are used."
  (unless (and (null position) (vectorp key) (= (length key) 0))
    (when (and (null position) (vectorp key))
      ;; A mouse click's own position, past a symbolic prefix such as
      ;; `mode-line'.
      (let ((event (aref key (if (and (symbolp (aref key 0)) (> (length key) 1)) 1 0))))
        (when (and (consp event) (consp (cdr event))
                   (eq (get (car event) 'event-kind) 'mouse-click))
          (setq position (event-start event)))))
    (let ((value (lookup-key (current-active-maps t position) key accept-default)))
      (unless (or (null value) (integerp value))
        (or (and (null no-remap) (symbolp value)
                 (command-remapping value position))
            value)))))

(provide 'romacs-keymap)
;;; romacs-keymap.el ends here
