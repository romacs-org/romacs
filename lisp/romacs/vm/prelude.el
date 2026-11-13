;;; prelude.el --- VM2 T3 tier: loops that call Lisp  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A Rust primitive never calls Lisp from a loop (PRIMITIVE-DOCTRINE section 3).
;; A loop whose body calls Lisp is written here instead, as ordinary compiled
;; Lisp over leaf cursors (`car'/`cdr', `aref', `length'), the same shape as the
;; `maphash' expansion over `#:vm2-hash-next' in expand.rs.  Every value such a
;; loop keeps across a callback is then a register of a traced Lisp frame by
;; construction, and a collection inside the callback forwards it.

;;; Code:

;; GNU fns.c's mapping functions.  Each takes the length first, so an improper
;; or circular list signals before FUNCTION runs, then walks as `mapcar1': a
;; list until the length or a tail FUNCTION shortened, anything else by index.
;; Each is its own loop, so a callback's frame sits directly on the mapping
;; function's, as on GNU's subr.

(defun romacs--map-indexed (sequence)
  "Refuse, as fns.c does after `length', a non-list SEQUENCE no mapping
function walks: a char-table is (wrong-type-argument listp SEQUENCE), and
anything but a vector, string or bool-vector (a record) is mapcar1's
(wrong-type-argument sequencep SEQUENCE)."
  (cond ((or (vectorp sequence) (stringp sequence) (bool-vector-p sequence)))
        ((char-table-p sequence) (signal 'wrong-type-argument (list 'listp sequence)))
        (t (signal 'wrong-type-argument (list 'sequencep sequence)))))

(defun mapcar (function sequence)
  "Apply FUNCTION to each element of SEQUENCE, and make a list of the results.
GNU fns.c `Fmapcar'."
  (let ((n (length sequence))
        (i 0)
        (acc nil))
    (if (listp sequence)
        (let ((tail sequence))
          (while (and (< i n) (consp tail))
            (setq acc (cons (funcall function (car tail)) acc))
            (setq tail (cdr tail))
            (setq i (1+ i))))
      (romacs--map-indexed sequence)
      (while (< i n)
        (setq acc (cons (funcall function (aref sequence i)) acc))
        (setq i (1+ i))))
    (nreverse acc)))

(defun mapc (function sequence)
  "Apply FUNCTION to each element of SEQUENCE for side effects only.
Return SEQUENCE.  GNU fns.c `Fmapc'."
  (let ((n (length sequence))
        (i 0))
    (if (listp sequence)
        (let ((tail sequence))
          (while (and (< i n) (consp tail))
            (funcall function (car tail))
            (setq tail (cdr tail))
            (setq i (1+ i))))
      (romacs--map-indexed sequence)
      (while (< i n)
        (funcall function (aref sequence i))
        (setq i (1+ i))))
    sequence))

(defun mapcan (function sequence)
  "Apply FUNCTION to each element of SEQUENCE, and `nconc' the results.
GNU fns.c `Fmapcan'."
  (let ((n (length sequence))
        (i 0)
        (acc nil))
    (if (listp sequence)
        (let ((tail sequence))
          (while (and (< i n) (consp tail))
            (setq acc (cons (funcall function (car tail)) acc))
            (setq tail (cdr tail))
            (setq i (1+ i))))
      (romacs--map-indexed sequence)
      (while (< i n)
        (setq acc (cons (funcall function (aref sequence i)) acc))
        (setq i (1+ i))))
    (romacs--nconc-list (nreverse acc))))

(defun mapconcat (function sequence &optional separator)
  "Apply FUNCTION to each element of SEQUENCE, and concat the results.
SEPARATOR goes between each pair of results.  GNU fns.c `Fmapconcat':
the symbol `identity' over a list or vector concatenates the elements
without calling it, and a nil or empty SEPARATOR inserts nothing.  The
join, separator included, is one call of `romacs--concat-list'."
  (let ((n (length sequence))
        (i 0)
        (parts nil))
    (if (and (eq function 'identity) (or (consp sequence) (vectorp sequence)))
        (setq parts (if (consp sequence) sequence (append sequence nil)))
      (if (listp sequence)
          (let ((tail sequence))
            (while (and (< i n) (consp tail))
              (setq parts (cons (funcall function (car tail)) parts))
              (setq tail (cdr tail))
              (setq i (1+ i))))
        (romacs--map-indexed sequence)
        (while (< i n)
          (setq parts (cons (funcall function (aref sequence i)) parts))
          (setq i (1+ i))))
      (setq parts (nreverse parts)))
    (romacs--concat-list parts separator)))


;;; Printing

(defun romacs--print-unreadable (function objects escape)
  "print.c print_vectorlike_unreadable: FUNCTION's value for each of the
unreadable OBJECTS a print meets, called with ESCAPE while
`print-unreadable-function' is nil, so a print it makes uses none."
  (let ((print-unreadable-function nil) (answers nil))
    (while objects
      (setq answers (cons (funcall function (car objects) escape) answers))
      (setq objects (cdr objects)))
    (nreverse answers)))

;;; Reverse keymap lookup

(defvar where-is-preferred-modifier nil)
(defvar minor-mode-overriding-map-alist nil)
(defvar emulation-mode-map-alists nil)

(defun romacs--where-append (prefix event)
  ;; Chartab walkers reuse the range cons between callbacks.
  (if (consp event) (setq event (cons (car event) (cdr event))))
  (let ((last (1- (length prefix))))
    (if (and (>= last 0) (eq (aref prefix last) meta-prefix-char)
             (integerp event))
        (let ((sequence (copy-sequence prefix)))
          (aset sequence last (logior event 134217728))
          sequence)
      (vconcat prefix (vector event)))))

(defun keymap--get-keyelt (object autoload)
  "Given OBJECT which was found in a slot in a keymap,
trace indirect definitions to get the actual definition of that slot.
Also if OBJECT has a menu string as the first element,
remove that.  Also remove a menu help string as second element.
If AUTOLOAD, apply a `menu-item''s `:filter' to its definition.
keymap.c get_keyelt; a circular wrapper, which GNU follows forever,
signals."
  (let ((done nil) (seen nil))
    (while (and (consp object) (not done))
      (if (memq object seen) (error "Circular menu item"))
      (setq seen (cons object seen))
      (cond ((eq (car object) 'menu-item)
             (if (not (consp (cdr object)))
                 (setq done t)
               (setq object (cdr (cdr object)))
               (let ((tail object))
                 (if (consp object) (setq object (car object)))
                 (if autoload
                     (while (and (consp tail) (consp (cdr tail)))
                       (if (not (eq (car tail) :filter))
                           (setq tail (cdr tail))
                         ;; keyboard.c menu_item_eval_property: an error is nil.
                         (let ((filter (car (cdr tail))) (inhibit-redisplay t))
                           (setq object (condition-case nil (eval (list filter (list 'quote object)) nil)
                                          (error nil))))
                         (setq tail nil)))))))
            ((stringp (car object)) (setq object (cdr object)))
            (t (setq done t))))
    object))

(defun romacs--keymap-noload (object)
  "keymap.c get_keymap without autoloading or signalling: the keymap
OBJECT is or names, OBJECT itself when it names an autoloaded keymap,
else nil."
  (if (eq (car-safe object) 'keymap) object
    (let ((function (and object (indirect-function object))))
      (cond ((eq (car-safe function) 'keymap) function)
            ((and (symbolp object) (eq (car-safe function) 'autoload)
                  (eq (nth 4 function) 'keymap))
             object)))))

(defun romacs--accessible-cycle-p (map sequence maps)
  "Whether MAP is already in MAPS under a prefix of SEQUENCE."
  (let ((cycle nil) entry)
    (while (and (not cycle) (setq entry (rassq map maps)))
      (let* ((prefix (car entry)) (n (length prefix)) (i 0))
        (if (<= n (length sequence))
            (progn
              (while (and (< i n) (eq (aref prefix i) (aref sequence i)))
                (setq i (1+ i)))
              (setq cycle (>= i n)))))
      (setq maps (cdr (memq entry maps))))
    cycle))

(defun accessible-keymaps (keymap &optional prefix)
  "Find all keymaps accessible via prefix characters from KEYMAP.
Returns a list of elements of the form (KEYS . MAP), where the sequence
KEYS starting from KEYMAP gets you to MAP.  These elements are ordered
so that the KEYS increase in length.  The first element is ([] . KEYMAP).
An optional argument PREFIX, if non-nil, should be a key sequence;
then the value includes only maps for prefixes that start with PREFIX.
keymap.c Faccessible_keymaps: a breadth-first walk that queues each
keymap bound to a prefix not already reached by a prefix of it."
  (let ((prefix-length (length prefix)) (maps nil))
    (if (not prefix)
        (let ((map (romacs--keymap-noload keymap)))
          (if (not (consp map)) (signal 'wrong-type-argument (list 'keymapp keymap)))
          (setq maps (list (cons [] map))))
      (let ((map (romacs--keymap-noload (lookup-key keymap prefix t))))
        (if map
            (progn
              ;; A string's 8-bit characters are meta characters.
              (if (stringp prefix)
                  (setq prefix (vconcat (mapcar (lambda (c)
                                                  (if (and (< c 256) (/= 0 (logand c 128)))
                                                      (logior (logxor c 128) 134217728)
                                                    c))
                                                prefix))))
              (setq maps (list (cons prefix map)))))))
    (let ((tail maps) (last maps))
      (while tail
        (let* ((sequence (car (car tail))) (map (cdr (car tail)))
               (end (1- (length sequence)))
               ;; Not the last event of PREFIX itself.
               (metized (and (>= end 0) (>= end prefix-length)
                             (eq (aref sequence end) meta-prefix-char))))
          ;; An autoloaded keymap cannot be walked without loading it.
          (if (consp map)
              (map-keymap
               (lambda (key binding)
                 (let ((child (romacs--keymap-noload (keymap--get-keyelt binding nil))))
                   (if (and child (not (romacs--accessible-cycle-p child sequence maps)))
                       (if (and metized (integerp key))
                           ;; The meta character's sequence is as long as
                           ;; SEQUENCE, so it goes right after it.
                           (let ((meta (copy-sequence sequence)))
                             (aset meta end (logior key 134217728))
                             (setcdr tail (cons (cons meta child) (cdr tail)))
                             (if (eq last tail) (setq last (cdr tail))))
                         ;; Chartab walkers reuse the range cons between callbacks.
                         (if (consp key) (setq key (cons (car key) (cdr key))))
                         (setcdr last (list (cons (vconcat sequence (vector key)) child)))
                         (setq last (cdr last))))))
               map)))
        (setq tail (cdr tail))))
    maps))

(defun romacs--where-bindings (maps)
  "Each key sequence MAPS bind, with its binding, in the order of
`accessible-keymaps' (keymap.c where_is_internal)."
  (let ((rows nil))
    (while (consp maps)
      (let ((prefixes (accessible-keymaps (car maps))))
        (while prefixes
          (let ((prefix (car (car prefixes))) (map (cdr (car prefixes))))
            (if (consp map)
                (map-keymap
                 (lambda (event raw)
                   (setq rows (cons (cons (romacs--where-append prefix event) raw) rows)))
                 map)))
          (setq prefixes (cdr prefixes))))
      (setq maps (cdr maps)))
    (nreverse rows)))

(defun romacs--where-lookup (maps sequence)
  (let ((value nil) (key (copy-sequence sequence)) (i 0))
    ;; GNU treats a range event as its first character during shadow lookup.
    (while (< i (length key))
      (let ((event (aref key i)))
        (if (and (consp event) (integerp (car event)) (integerp (cdr event)))
            (aset key i (car event))))
      (setq i (1+ i)))
    (while (and maps (not value))
      (setq value (lookup-key (car maps) key))
      ;; An absent prefix falls through; a defined non-prefix shadows later maps.
      (if (and (integerp value)
               (not (lookup-key (car maps) (substring key 0 value))))
          (setq value nil))
      (setq maps (cdr maps)))
    (if (integerp value) (setq value nil))
    value))

(defun romacs--where-remapping (definition maps)
  (if (symbolp definition)
      (romacs--where-lookup maps (vector 'remap definition))))

(defun romacs--where-mouse-prefix-p (sequence)
  (let ((event (if (> (length sequence) 1) (aref sequence 0))))
    (if (symbolp event)
        (let ((name (symbol-name event)) (continue t))
          (while continue
            (let ((i 0))
              (while (and (< i (length name)) (/= (aref name i) 45))
                (setq i (1+ i)))
              (if (and (< i (length name)) (member (substring name 0 i)
                          '("A" "C" "H" "M" "S" "s" "up" "down" "drag"
                            "click" "double" "triple")))
                  (setq name (substring name (1+ i)))
                (setq continue nil))))
          (member name '("menu-bar" "tab-bar" "tool-bar" "tab-line"
                         "header-line" "mode-line" "mouse-1" "mouse-2"
                         "mouse-3" "mouse-4" "mouse-5"))))))

(defun romacs--where-candidates (definition rows noindirect nomenus)
  (let ((found nil))
    (while rows
      (let* ((row (car rows)) (binding (cdr row)))
        (if (not noindirect) (setq binding (keymap--get-keyelt binding nil)))
        (if (and (or (eq binding definition)
                     (and (consp definition) (equal binding definition)))
                 (not (and nomenus (romacs--where-mouse-prefix-p (car row)))))
            (setq found (cons (car row) found))))
      (setq rows (cdr rows)))
    (nreverse found)))

(defun romacs--where-preferred (sequence modifier)
  (let ((i 0) (result 1))
    (while (and (> result 0) (< i (length sequence)))
      (let ((event (aref sequence i)))
        (if (not (integerp event)) (setq result 0)
          (let ((bits (logand event 130023424)))
            (if (= bits modifier) (setq result 2)
              (if (/= bits 0) (setq result 0))))))
      (setq i (1+ i)))
    result))

(defun romacs--where-modifier ()
  (let ((entry (assq where-is-preferred-modifier
                     '((A . 4194304) (alt . 4194304) (C . 67108864)
                       (ctrl . 67108864) (control . 67108864) (H . 16777216)
                       (hyper . 16777216) (M . 134217728) (meta . 134217728)
                       (S . 33554432) (shift . 33554432) (s . 8388608)
                       (super . 8388608) (up . 1) (down . 2) (drag . 4)
                       (click . 8) (double . 16) (triple . 32)))))
    (if entry (cdr entry) 0)))

(defun romacs--where-advertised (definition maps)
  (let ((advertised (if (symbolp definition) (get definition :advertised-binding)))
        (result nil))
    (while (and (consp advertised) (not result))
      (if (eq (romacs--where-lookup maps (car advertised)) definition)
          (setq result (car advertised)))
      (setq advertised (cdr advertised)))
    (if (and (not result) advertised
             (eq (romacs--where-lookup maps advertised) definition))
        (setq result advertised))
    result))

(defvar romacs--where-cache nil
  "keymap.c where_is_cache: each definition's key sequences in the keymaps
`romacs--where-cache-maps', ignoring menus and following indirection, while
`romacs--keymap-generation' stays `romacs--where-cache-generation'.")
(defvar romacs--where-cache-maps t)
(defvar romacs--where-cache-generation nil)

(defun romacs--where-cached (maps)
  "The reverse map of MAPS, rebuilt when they or any keymap changed."
  (if (not (and romacs--where-cache (equal maps romacs--where-cache-maps)
                (eql romacs--where-cache-generation (romacs--keymap-generation))))
      (let ((table (make-hash-table :test 'eql))
            (rows (romacs--where-bindings maps)))
        (while rows
          (let ((row (car rows)))
            (if (not (romacs--where-mouse-prefix-p (car row)))
                (let ((binding (keymap--get-keyelt (cdr row) nil)))
                  (puthash binding (cons (car row) (gethash binding table)) table))))
          (setq rows (cdr rows)))
        (maphash (lambda (binding sequences) (puthash binding (nreverse sequences) table)) table)
        (setq romacs--where-cache table romacs--where-cache-maps maps
              romacs--where-cache-generation (romacs--keymap-generation))))
  romacs--where-cache)

(defun where-is-internal (definition &optional keymap firstonly noindirect no-remap)
  "Return keys invoking DEFINITION in KEYMAP, respecting shadows and remapping."
  (let* ((maps (cond ((and (consp keymap) (keymapp (car keymap))) keymap)
                     (keymap (list keymap (current-global-map)))
                     (t (current-active-maps))))
         (remapping (romacs--where-remapping definition maps))
         (modifier (romacs--where-modifier)))
    (if (and (not no-remap) remapping) (setq definition remapping))
    (or (and firstonly (romacs--where-advertised definition maps))
        (romacs--where-find definition maps firstonly noindirect no-remap modifier))))

(defun romacs--where-find (definition maps firstonly noindirect no-remap modifier)
  (let* ((nomenus (and firstonly (not (eq firstonly 'non-ascii))))
         ;; keymap.c where_is_internal: the cache serves menu-less lookups
         ;; through indirection; any other lookup enumerates the maps.
         (cache (and nomenus (not noindirect) (romacs--where-cached maps)))
         (rows (if (not cache) (romacs--where-bindings maps)))
         (candidates (lambda (definition)
                       (if cache (gethash definition cache)
                         (romacs--where-candidates definition rows noindirect nomenus))))
         (pending (funcall candidates definition))
         (remapped nil) (second nil) (found nil) (answer nil))
    (while (and (or pending (not second)) (not answer))
      (if (not pending) (setq pending remapped second t))
      (if pending
          (let* ((sequence (car pending))
                 (value (romacs--where-lookup maps sequence)))
            (setq pending (cdr pending))
            (if (and second (symbolp value) value)
                (setq value (or (romacs--where-remapping value maps) value)))
            (if (equal value definition)
                (if (and (not no-remap) (not second) (= (length sequence) 2)
                         (eq (aref sequence 0) 'remap) (symbolp (aref sequence 1)))
                    (setq remapped (append (funcall candidates (aref sequence 1)) remapped))
                  ;; The cache shares SEQUENCE: change a copy.
                  (if (stringp (aref sequence (1- (length sequence))))
                      (progn (setq sequence (copy-sequence sequence))
                             (aset sequence (1- (length sequence)) "(any string)")))
                  (if (not (or (member sequence found)
                               (and (= (length sequence) 1)
                                    (symbolp (aref sequence 0))
                                    (get (aref sequence 0) 'non-key-event))))
                      (setq found (cons sequence found)))
                  (if (or (eq firstonly 'non-ascii)
                          (and firstonly (= (romacs--where-preferred sequence modifier) 2)))
                      (setq answer sequence)))))))
    (setq found (nreverse found))
    (or answer (if (not firstonly) found
                 (romacs--where-first found modifier)))))

(defun romacs--where-first (found modifier)
  (let ((tail found) (answer nil))
    (while (and (/= modifier 0) tail (not answer))
      (if (> (romacs--where-preferred (car tail) modifier) 0)
          (setq answer (car tail)))
      (setq tail (cdr tail)))
    (or answer (car found))))

;; textprop.c's char-property motion, composed as its C composes the
;; overlay and text-property primitives.

(defun romacs--fixnum-coerce-marker (value)
  "CHECK_FIXNUM_COERCE_MARKER: VALUE as a fixnum, a marker's position."
  (cond ((markerp value)
         (or (marker-position value) (error "Marker does not point anywhere")))
        ((integerp value) value)
        (t (signal 'wrong-type-argument (list 'integer-or-marker-p value)))))

(defun next-char-property-change (position &optional limit)
  "Return the position of next text property or overlay change.
textprop.c Fnext_char_property_change: the nearer of the next overlay
change and LIMIT bounds `next-property-change'."
  (let ((temp (next-overlay-change position)))
    (if limit
        (let ((limit (romacs--fixnum-coerce-marker limit)))
          (if (< limit temp) (setq temp limit))))
    (next-property-change position nil temp)))

(defun previous-char-property-change (position &optional limit)
  "Return the position of previous text property or overlay change.
textprop.c Fprevious_char_property_change."
  (let ((temp (previous-overlay-change position)))
    (if limit
        (let ((limit (romacs--fixnum-coerce-marker limit)))
          (if (> limit temp) (setq temp limit))))
    (previous-property-change position nil temp)))

;; buffer.c's `find-buffer'.
(defun find-buffer (variable value)
  "Return the buffer with buffer-local VARIABLE `equal' to VALUE.
If there is no such live buffer, return nil.
See also `find-buffer-visiting'."
  (catch 'romacs--find-buffer
    (dolist (buffer (buffer-list))
      (when (equal value (buffer-local-value variable buffer))
        (throw 'romacs--find-buffer buffer)))
    nil))

;; eval.c's `buffer-local-toplevel-value' and its setter.

(defun buffer-local-toplevel-value (symbol &optional buffer)
  "Return SYMBOL's toplevel buffer-local value in BUFFER.
\"Toplevel\" means outside of any let binding.
BUFFER defaults to the current buffer.
If SYMBOL has no local value in BUFFER, signals an error."
  (let ((buffer (or buffer (current-buffer))))
    (unless (local-variable-p symbol buffer) (signal 'void-variable (list symbol)))
    (let ((cell (romacs--local-toplevel-cell symbol buffer)))
      (if cell (cdr cell) (buffer-local-value symbol buffer)))))

(defun set-buffer-local-toplevel-value (symbol value &optional buffer)
  "Set SYMBOL's toplevel buffer-local value in BUFFER to VALUE.
\"Toplevel\" means outside of any let-binding.
BUFFER defaults to the current buffer.
Makes SYMBOL buffer-local in BUFFER if it was not already."
  (unless (romacs--set-local-toplevel-cell symbol (or buffer (current-buffer)) value)
    (if buffer
        (save-current-buffer (set-buffer buffer) (set (make-local-variable symbol) value))
      (set (make-local-variable symbol) value)))
  nil)

;; window.c's `minibuffer-selected-window'. minibuf.el, loaded on the
;; first minibuffer read, records the window as read_minibuf does.

(defvar romacs--minibuf-selected-window nil)

(defun minibuffer-selected-window ()
  "Return window selected just before minibuffer window was selected.
Return nil if the selected window is not a minibuffer window."
  (and (boundp 'romacs--minibuf-level)
       (> romacs--minibuf-level 0)
       (window-minibuffer-p (selected-window))
       (window-live-p romacs--minibuf-selected-window)
       romacs--minibuf-selected-window))

;; eval.c's `func-arity' and its `lambda_arity'.  Closure parameter
;; counts come from the compiled function.

(defun romacs--lambda-arity (fun)
  (unless (consp (cdr fun)) (signal 'invalid-function (list fun)))
  (let ((args (car (cdr fun))) (min 0) (max 0) optional result)
    (while (and (consp args) (not result))
      (let ((next (car args)))
        (cond ((not (symbolp next)) (signal 'invalid-function (list fun)))
              ((eq next '&rest) (setq result (cons min 'many)))
              ((eq next '&optional) (setq optional t))
              (t (unless optional (setq min (1+ min)))
                 (setq max (1+ max)))))
      (setq args (cdr args)))
    (cond (result)
          (args (signal 'invalid-function (list fun)))
          (t (cons min max)))))

(defun func-arity (function)
  "Return minimum and maximum number of args allowed for FUNCTION.
FUNCTION must be a function of some kind.
The returned value is a cons cell (MIN . MAX).  MIN is the minimum number
of args.  MAX is the maximum number, or the symbol `many', for a
function with `&rest' args, or `unevalled' for a special form."
  (let ((original function) result)
    (while (not result)
      (setq function original)
      (when (and (symbolp function) function)
        (setq function (symbol-function function))
        (when (symbolp function) (setq function (indirect-function function))))
      (when (and (consp function) (eq (car function) 'macro))
        (setq function (cdr function)))
      (cond ((subrp function) (setq result (subr-arity function)))
            ((closurep function) (setq result (romacs--closure-arity function)))
            ((null function) (signal 'void-function (list original)))
            ((not (and (consp function) (symbolp (car function))))
             (signal 'invalid-function (list original)))
            ((eq (car function) 'lambda)
             (setq result (romacs--lambda-arity function)))
            ((eq (car function) 'autoload)
             (autoload-do-load function original nil))
            (t (signal 'invalid-function (list original)))))
    result))

;; buffer.c's `set-buffer-major-mode' and editfns.c's
;; `insert-buffer-substring', composed of the primitives their C calls.

(defun set-buffer-major-mode (buffer)
  "Set an appropriate major mode for BUFFER.
For the *scratch* buffer, use `initial-major-mode', otherwise choose a mode
according to the default value of `major-mode'.
Use this function before selecting the buffer, since it may need to inspect
the current buffer's major mode."
  (unless (bufferp buffer) (signal 'wrong-type-argument (list 'bufferp buffer)))
  (unless (buffer-live-p buffer) (error "Attempt to set major mode for a dead buffer"))
  (let ((function (if (equal (buffer-name buffer) "*scratch*")
                      (symbol-value 'initial-major-mode)
                    (let ((default (default-value 'major-mode)))
                      (if (and (null default) (null (get major-mode 'mode-class)))
                          major-mode
                        default)))))
    (when function
      (save-current-buffer
        (set-buffer buffer)
        (funcall function)))
    nil))

(defun romacs--pre-redisplay (windows)
  "xdisp.c prepare_menu_bars's call of `pre-redisplay-function' on WINDOWS,
as safe_call makes it."
  (let ((inhibit-redisplay t)
        (inhibit-quit t))
    (condition-case err
        (funcall pre-redisplay-function windows)
      (error (message "Error during redisplay: %S" err)))))

(defun romacs--native-region (_start _end _window _overlay)
  "`redisplay-highlight-region-function' while Romacs draws the region itself."
  nil)

(defun insert-buffer-substring (buffer &optional start end)
  "Insert before point a substring of the contents of BUFFER.
BUFFER may be a buffer or a buffer name.
Arguments START and END are character positions specifying the substring.
They default to the values of (point-min) and (point-max) in BUFFER.

Point and before-insertion markers move forward to end up after the
inserted text.
Any other markers at the point of insertion remain before the text.

If the current buffer is multibyte and BUFFER is unibyte, or vice
versa, strings are converted from unibyte to multibyte or vice versa
using `string-make-multibyte' or `string-make-unibyte', which see."
  (let ((buf (get-buffer buffer)))
    (unless buf
      (if (stringp buffer)
          (error "No buffer named %s" buffer)
        (error "Invalid buffer argument")))
    (unless (buffer-live-p buf) (error "Selecting deleted buffer"))
    (insert
     (save-current-buffer
       (set-buffer buf)
       (let ((b (if start (romacs--fixnum-coerce-marker start) (point-min)))
             (e (if end (romacs--fixnum-coerce-marker end) (point-max))))
         (when (> b e) (setq b (prog1 e (setq e b))))
         (unless (and (<= (point-min) b) (<= e (point-max)))
           (signal 'args-out-of-range (list start end)))
         ;; update_buffer_properties runs these before the text is read.
         (when (and (boundp 'buffer-access-fontify-functions) buffer-access-fontify-functions)
           (error "VM2 insert-buffer-substring with buffer-access-fontify-functions is unsupported"))
         (buffer-substring b e))))
    nil))

;; coding.c's region conversions: code_convert_region over the string
;; conversions, and detect-coding-region over detect-coding-string.

(defun romacs--coding-insert (buffer text)
  "Insert TEXT at BUFFER's point, which stays before it; return its length.
The insertion decode_coding_object and encode_coding_object make into
a buffer other than the source."
  (with-current-buffer buffer
    (save-excursion (insert text)))
  (length text))

(defun detect-coding-region (start end &optional highest)
  "Detect the coding system of the text between START and END.
coding.c Fdetect_coding_region: the possible coding systems in priority
order, or with HIGHEST the first."
  (detect-coding-string (buffer-substring-no-properties start end) highest))

;; fns.c's `secure-hash' and `md5': extract_data_from_object chooses and
;; encodes the data, and `romacs--secure-hash' digests it.

(defun romacs--hash-data (object start end coding-system noerror)
  "The encoded bytes of OBJECT from START to END to digest.
fns.c extract_data_from_object."
  (cond
   ((stringp object)
    (unless coding-system
      (setq coding-system (if (multibyte-string-p object)
                              (coding-system-priority-list t)
                            'raw-text)))
    (unless (coding-system-p coding-system)
      (if noerror
          (setq coding-system 'raw-text)
        (signal 'coding-system-error (list coding-system))))
    (when (multibyte-string-p object)
      (let ((last-coding-system-used last-coding-system-used))
        (setq object (encode-coding-string object coding-system))))
    (substring-no-properties object start end))
   ((bufferp object)
    (let ((text
           (with-current-buffer object
             (let ((b (if start (romacs--fixnum-coerce-marker start) (point-min)))
                   (e (if end (romacs--fixnum-coerce-marker end) (point-max))))
               (when (> b e) (setq b (prog1 e (setq e b))))
               (unless (and (<= (point-min) b) (<= e (point-max)))
                 (signal 'args-out-of-range (list start end)))
               (unless coding-system
                 ;; As write-region decides it.
                 (if coding-system-for-write
                     (setq coding-system coding-system-for-write)
                   (let ((force-raw-text nil))
                     (setq coding-system buffer-file-coding-system)
                     (when (or (null coding-system)
                               (not (local-variable-p 'buffer-file-coding-system)))
                       (setq coding-system nil)
                       (unless enable-multibyte-characters (setq force-raw-text t)))
                     (when (and (null coding-system) (buffer-file-name object))
                       (let ((val (find-operation-coding-system
                                   'write-region b e (buffer-file-name object))))
                         (when (and (consp val) (cdr val)) (setq coding-system (cdr val)))))
                     (when (and (null coding-system) buffer-file-coding-system)
                       (setq coding-system buffer-file-coding-system))
                     (when (and (not force-raw-text)
                                (boundp 'select-safe-coding-system-function)
                                (fboundp select-safe-coding-system-function))
                       (setq coding-system (funcall select-safe-coding-system-function
                                                    b e coding-system nil)))
                     (when force-raw-text (setq coding-system 'raw-text))))
                 (unless (coding-system-p coding-system)
                   (if noerror
                       (setq coding-system 'raw-text)
                     (signal 'coding-system-error (list coding-system)))))
               (buffer-substring-no-properties b e)))))
      (if (multibyte-string-p text)
          (encode-coding-string text coding-system)
        text)))
   ((eq object 'iv-auto)
    (if (natnump start)
        (error "secure-hash `iv-auto' random data is not implemented")
      (error "Without a length, `iv-auto' can't be used; see Elisp manual")))
   (t (signal 'error (cons "Invalid object argument"
                           (if (consp object) object (list (or object "nil"))))))))

(defun secure-hash (algorithm object &optional start end binary)
  "Return the secure hash of OBJECT, a buffer or string.
ALGORITHM is a symbol specifying the hash to use:
- md5      corresponds to MD5, produces a 32-character signature
- sha1     corresponds to SHA-1, produces a 40-character signature
- sha224   corresponds to SHA-2 (SHA-224), produces a 56-character signature
- sha256   corresponds to SHA-2 (SHA-256), produces a 64-character signature
- sha384   corresponds to SHA-2 (SHA-384), produces a 96-character signature
- sha512   corresponds to SHA-2 (SHA-512), produces a 128-character signature
- sha3-224 corresponds to SHA-3 (SHA3-224), produces a 56-character signature
- sha3-256 corresponds to SHA-3 (SHA3-256), produces a 64-character signature
- sha3-384 corresponds to SHA-3 (SHA3-384), produces a 96-character signature
- sha3-512 corresponds to SHA-3 (SHA3-512), produces a 128-character signature

The two optional arguments START and END are positions specifying for
which part of OBJECT to compute the hash.  If nil or omitted, uses the
whole OBJECT.

The full list of algorithms can be obtained with `secure-hash-algorithms'.

If BINARY is non-nil, returns a string in binary form.  In this case,
the function returns a unibyte string whose length is half the number
of characters it returns when BINARY is nil."
  (unless (symbolp algorithm) (signal 'wrong-type-argument (list 'symbolp algorithm)))
  (romacs--secure-hash algorithm (romacs--hash-data object start end nil nil) binary))

(defun md5 (object &optional start end coding-system noerror)
  "Return MD5 message digest of OBJECT, a buffer or string.
The two optional arguments START and END are character positions
specifying for which part of OBJECT the message digest should be
computed.  CODING-SYSTEM encodes the text; by default it is what writing
a buffer would use, or the most preferred coding system for a string.
If NOERROR is non-nil, silently assume the `raw-text' coding if the
guesswork fails."
  (romacs--secure-hash 'md5 (romacs--hash-data object start end coding-system noerror)))

;; GNU safe_calln (eval.c): a signal from FUNCTION is muted, logged to
;; *Messages* only (add_to_log), and the call returns nil.  syntax.c calls
;; `internal--syntax-propertize' this way in the middle of a scan.
(defun romacs--safe-call (function &rest args)
  (romacs--muted-call "Error muted by safe_call" function args))

(defun romacs--muted-call (log function args)
  "Apply FUNCTION to ARGS with redisplay inhibited; a signal returns nil.
The signal is logged, after LOG, to *Messages* only, as GNU's add_to_log."
  (let ((inhibit-redisplay t))
    (condition-case err (apply function args)
      (t (let ((inhibit-message t))
           (message "%s" (format "%s: %S signaled %S" log (cons function args) err)))
         nil))))

;; GNU handle_fontified_prop (xdisp.c): where the text redisplay shows has a
;; nil `fontified' char property, call `fontification-functions' with that
;; position, each through dsafe_call1 and with the variable itself bound to
;; nil.  A buffer-local `t' runs the global functions; a global `t' is
;; ignored.  Redisplay then reads the property again: a function that left
;; it nil is not called again in that stretch, so none can loop.
(defun romacs--redisplay-fontify (start end)
  "Run `fontification-functions' over the unfontified text from START to END.
Redisplay calls this in the buffer of a window showing START to END."
  (let ((buffer (current-buffer)) (pos start))
    (while (and pos (< pos end) fontification-functions (not memory-full))
      (if (get-char-property pos 'fontified)
          (setq pos (next-single-char-property-change pos 'fontified nil end))
        (romacs--run-fontification-functions pos)
        (if (not (buffer-live-p buffer))
            (setq pos nil)
          ;; GNU's fig leaf: back to the displayed buffer, whose text the
          ;; functions may have shortened.
          (set-buffer buffer)
          (setq end (min end (point-max)))
          (unless (get-char-property pos 'fontified)
            (setq pos (next-single-char-property-change pos 'fontified nil end))))))))

(defun romacs--run-fontification-functions (pos)
  (let ((functions fontification-functions)
        (fontification-functions nil))
    (if (or (not (consp functions)) (eq (car functions) 'lambda))
        (romacs--muted-call "Error during redisplay" functions (list pos))
      (while (consp functions)
        (if (not (eq (car functions) t))
            (romacs--muted-call "Error during redisplay" (car functions) (list pos))
          (let ((globals (default-value 'fontification-functions)))
            (while (consp globals)
              (unless (eq (car globals) t)
                (romacs--muted-call "Error during redisplay" (car globals) (list pos)))
              (setq globals (cdr globals)))))
        (setq functions (cdr functions))))))


;;; Evaluating in a frame

(defun backtrace-eval (exp nframes &optional base)
  "Evaluate EXP in the context of some activation frame.
NFRAMES and BASE specify the activation frame to use, as in `backtrace-frame'.
eval.c Fbacktrace_eval: the dynamic bindings, the current buffer and the
excursions made since that frame's entry are swapped back while EXP runs,
and forward again however it exits."
  (let ((swapped (romacs--backtrace-unrewind nframes (or base 'backtrace-eval))))
    (unwind-protect (eval exp t)
      (romacs--backtrace-rewind swapped))))

;;; The debugger on the signal path

;; GNU eval.c maybe_call_debugger.  The signal path (threaded_signal.rs
;; debugger_wanted) has already found the debugger considered for the error,
;; `inhibit-debugger' nil and `debug-on-quit' or `debug-on-error' non-nil;
;; when this says yes, it calls `debugger' itself under call_debugger's
;; bindings (threaded.rs call_debugger), so no frame of this file's sits
;; between the debugger and the error.
(defun romacs--call-debugger-p (conditions error)
  "Whether to call `debugger' for ERROR, whose CONDITIONS are its
`error-conditions': unless `debug-on-error' lists none of them,
`debug-ignored-errors' matches it, or the debugger already ran for the
current input event.  Yes notes the event, as call_debugger does."
  (when (and (or (memq 'quit conditions)
                 (not (consp debug-on-error))
                 (let ((tail conditions) (found nil))
                   (while (and (consp tail) (not found))
                     (setq found (memq (car tail) debug-on-error)
                           tail (cdr tail)))
                   found))
             (not (romacs--skip-debugger conditions error))
             (< internal-when-entered-debugger num-nonmacro-input-events))
    (setq debug-on-next-call nil
          internal-when-entered-debugger num-nonmacro-input-events)
    t))

(defun romacs--skip-debugger (conditions error)
  "eval.c skip_debugger: whether `debug-ignored-errors' names one of
CONDITIONS or has a regexp matching ERROR's message."
  (let ((tail debug-ignored-errors) (message nil) (skip nil))
    (while (and (consp tail) (not skip))
      (if (stringp (car tail))
          (setq message (or message (error-message-string error))
                skip (string-match (car tail) message nil t))
        (setq skip (memq (car tail) conditions)))
      (setq tail (cdr tail)))
    skip))

;;; Finalizers

(defvar romacs--undo-offering nil
  "Non-nil while `romacs--offer-undo-lists' asks: a collection inside the
function, which GNU's has inhibited, asks nothing more of it.")

(defun romacs--offer-undo-lists ()
  "undo.c truncate_undo_list's call of `undo-outer-limit-function', for each
buffer whose first undo record a collection found past `undo-outer-limit'.
A collection runs no Lisp, so this comes after it (VM2-UNDO-OUTER), with that
buffer current.  A nil answer truncates the list as without the function; any
other leaves it to the function, and so does an error, logged to *Messages*.
A quit leaves this buffer and the rest to the next collection."
  (unless romacs--undo-offering
    (let ((offer (romacs--take-undo-offer))
          (asked nil))
      (when offer
        (let ((romacs--undo-offering t)
              (match-data (match-data)))
          (save-current-buffer
            (unwind-protect
                ;; One at a time, each looked for afresh: the function, or
                ;; whatever ran while it asked, may have changed another
                ;; buffer's list since the collection.  A buffer is asked
                ;; about once in a run: one the function has put past the
                ;; limit again since is the next collection's.
                (while offer
                  (setq asked (cons (car offer) asked))
                  (set-buffer (car offer))
                  (romacs--answer-undo-offer
                   (car offer)
                   (condition-case err
                       (funcall undo-outer-limit-function (cdr offer))
                     (error (let ((inhibit-message t))
                              (message "undo-outer-limit-function failed: %S" err))
                            t)))
                  (setq offer (romacs--take-undo-offer asked)))
              (set-match-data match-data t))))))))

(defvar romacs--post-gc-owned nil
  "Non-nil while a `romacs--run-finalizers' that took a collection's
`post-gc-hook' is active: a run started by a collection inside it leaves
the hook to it.  Never buffer-local: a hook that switches buffer must
still see it.")

(defun romacs--run-finalizers ()
  "alloc.c run_finalizers: call the function of each finalizer the
collections since the last run found unreachable, once each, with quits
inhibited; a signal is logged to *Messages*, as `add_to_log' does.  Then
`post-gc-hook', if a collection ended since it last ran and the hook has a
function once the finalizers are done (one may have added it).  This run
owns that collection's hook from the start, so it runs after the finalizers.

Collection is not inhibited (VM2-POST-GC): a collection inside the run, in a
finalizer or the hook, is an ordinary one that runs its own finalizers; its
hook is this run's, which runs once.  The run takes its work before it
allocates, and a run with nothing to do returns at once.  It runs at a safe
point after the collection, inside whatever code runs there, and keeps that
code's match data and current buffer, as for a process filter."
  ;; As garbage_collect, the undo lists come first.  Theirs is no finalizer's
  ;; work: quits are not held off from a function that may ask a question.
  (romacs--offer-undo-lists)
  ;; Quits wait from before anything is taken: a finalizer taken is emptied.
  (let* ((inhibit-quit t)
         (collected (and (not romacs--post-gc-owned) (romacs--take-post-gc)))
         (function (romacs--take-doomed-finalizer)))
    (when (or function (and collected post-gc-hook))
      (let* ((romacs--post-gc-owned (or romacs--post-gc-owned collected))
             (match-data (match-data)))
        (save-current-buffer
          (unwind-protect
              (progn
                (while function
                  (condition-case err
                      (funcall function)
                    (t (let ((inhibit-message t))
                         (message "finalizer failed: %S" err))))
                  (setq function (romacs--take-doomed-finalizer)))
                ;; As garbage_collect, the hook is looked at after the
                ;; finalizers: one may have added to it.
                (when (and collected post-gc-hook)
                  (unwind-protect (romacs--safe-run-hooks 'post-gc-hook)
                    ;; The collections that ended while it ran were its own.
                    (romacs--take-post-gc))))
            (set-match-data match-data t)))))))

;;; Evaluating buffer text: lread.c readevalloop

(defun romacs--lexical-cookie ()
  "lread.c lisp_file_lexical_cookie: `lex', `dyn' or nil, from the
`lexical-binding' file variable on the line at point (the next one after
a #! line).  Point is unchanged."
  (save-excursion
    (if (looking-at "#!") (forward-line 1))
    (let ((data (match-data)) (cookie nil))
      (unwind-protect
          (if (eq (char-after) ?\;)
              (let* ((line (buffer-substring-no-properties (point) (line-end-position)))
                     (vars (and (string-match "-\\*-\\(.*?\\)\\(?:-\\*-\\|\\'\\)" line)
                                (split-string (match-string 1 line) ";"))))
                (while vars
                  (if (string-match "\\`[ \t]*lexical-binding[ \t]*:[ \t]*\\(.*?\\)[ \t]*\\'" (car vars))
                      (setq cookie (if (equal (match-string 1 (car vars)) "nil") 'dyn 'lex)))
                  (setq vars (cdr vars)))))
        (set-match-data data))
      cookie)))

(defun romacs--skip-to-form ()
  "Skip the whitespace and `;' comments readevalloop skips before a
form; nil at the end of the accessible text."
  (let ((done nil))
    (while (not done)
      (skip-chars-forward " \t\n\f\r ")
      (if (eq (char-after) ?\;) (skip-chars-forward "^\n") (setq done t))))
  (not (eobp)))

(defun romacs--eager-eval (form expander environment)
  "lread.c readevalloop_eager_expand_eval: FORM expanded one level, each
form of a resulting `progn' as a top-level form, else fully, then
evaluated in ENVIRONMENT.  Return (VALUE . ENVIRONMENT): a top-level
`(defvar NAME)' adds NAME to a lexical environment, as Fdefvar does."
  (if expander (setq form (funcall expander form nil)))
  (if (and expander (eq (car-safe form) 'progn))
      (let ((result (cons nil environment)) (forms (cdr form)))
        (while (consp forms)
          (setq result (romacs--eager-eval (car forms) expander (cdr result)))
          (setq forms (cdr forms)))
        result)
    (if expander (setq form (funcall expander form t)))
    (let ((value (eval form environment)))
      (if (and environment (eq (car-safe form) 'defvar) (consp (cdr form)) (null (cdr (cdr form)))
               (symbolp (car (cdr form))) (not (special-variable-p (car (cdr form)))))
          (setq environment (cons (car (cdr form)) environment)))
      (cons value environment))))

(defun romacs--build-load-history (filename entire)
  "lread.c build_load_history: FILENAME's entry in `load-history' from
`current-load-list', replacing an old one when ENTIRE, else merged in."
  (let ((tail load-history) (previous nil) (found nil))
    (while (consp tail)
      (if (not (equal filename (car-safe (car tail))))
          (setq previous tail)
        (setq found t)
        (if entire
            (if previous (setcdr previous (cdr tail)) (setq load-history (cdr tail)))
          (let ((new current-load-list))
            (while (consp new)
              (if (not (member (car new) (car tail)))
                  (setcar tail (cons (car (car tail)) (cons (car new) (cdr (car tail))))))
              (setq new (cdr new))))))
      (setq tail (cdr tail)))
    (if (or entire (not found))
        (setq load-history (cons (reverse current-load-list) load-history)))))

(defun romacs--readevalloop-read (buffer readfun)
  "The next form of BUFFER at point for `romacs--readevalloop', as
(FORM . MORE), MORE nil when READFUN left point at the end; nil at the
end of the accessible text."
  (if (romacs--skip-to-form)
      (let ((form (cond (readfun (funcall readfun buffer))
                        (load-read-function (funcall load-read-function buffer))
                        (t (read buffer)))))
        (cons form (not (and readfun (= (point) (point-max))))))))

(defun romacs--readevalloop (buffer sourcename printflag readfun start end)
  "lread.c readevalloop over BUFFER: read each form with READFUN,
`load-read-function' or `read', and evaluate it as top-level code.  With
START, read from there up to END in BUFFER, as `eval-region' does; else
from BUFFER's point.  Print each value when PRINTFLAG; record what the
forms define in `load-history' under SOURCENAME."
  (let* ((standard-input buffer)
         (expander (and (fboundp 'internal-macroexpand-for-load) #'internal-macroexpand-for-load))
         (environment (if lexical-binding (list t)))
         (macroexp--dynvars macroexp--dynvars)
         (sourcename (if (and sourcename (file-name-absolute-p sourcename)) (expand-file-name sourcename) sourcename))
         (current-load-list (list sourcename))
         (whole 'unknown) (read t))
    (while read
      (if (not (buffer-live-p buffer)) (error "Reading from killed buffer"))
      (setq read
            (save-current-buffer
              (set-buffer buffer)
              (let ((at-start (lambda ()
                                (if (eq whole 'unknown)
                                    (setq whole (and (= (point) 1) (= (point-max) (1+ (buffer-size))))))
                                (romacs--readevalloop-read buffer readfun))))
                (if (not start)
                    ;; Reading BUFFER itself leaves its point after the form.
                    (funcall at-start)
                  ;; eval-region reads each form at START, narrowed to END,
                  ;; and keeps BUFFER's point and restriction.
                  (save-excursion
                    (save-restriction
                      (goto-char start)
                      (if end (narrow-to-region (point-min) end))
                      ;; An integer END becomes a marker, which follows
                      ;; the text the forms change.
                      (if (integerp end) (setq end (point-max-marker)))
                      (let ((read (funcall at-start)))
                        (if (cdr read) (setq start (point-marker)))
                        read)))))))
      (if read
          (let ((result (romacs--eager-eval (car read) expander environment)))
            (setq environment (cdr result))
            (if printflag
                (progn
                  (setq values (cons (car result) values))
                  (if (eq standard-output t) (prin1 (car result)) (print (car result)))))
            (if (not (cdr read)) (setq read nil)))))
    (romacs--build-load-history sourcename (eq whole t))))

(defun eval-buffer (&optional buffer printflag filename _unibyte do-allow-print)
  "Execute the accessible portion of current buffer as Lisp code.
When called from a Lisp program, BUFFER is the buffer to evaluate (nil
means the current buffer) or its name; PRINTFLAG nil discards the output
of output functions in the evaluated code, anything else is the stream to
print to; FILENAME names the code in `load-history'; DO-ALLOW-PRINT
non-nil lets output functions print normally even with PRINTFLAG nil.
lread.c Feval_buffer over `romacs--readevalloop'."
  (interactive "")
  (let ((buf (if buffer (get-buffer buffer) (current-buffer))))
    (if (null buf) (error "No such buffer"))
    (let ((standard-output (if (or printflag do-allow-print) printflag 'symbolp))
          (filename (or filename (buffer-local-value 'buffer-file-name buf)))
          (eval-buffer-list (cons buf eval-buffer-list)))
      (save-excursion
        (save-current-buffer
          (set-buffer buf)
          (goto-char (point-min)))
        ;; A buffer-local `lexical-binding' is kept: it, not the
        ;; cookie, names the dialect then.
        (if (local-variable-p 'lexical-binding buf)
            (romacs--readevalloop buf filename printflag nil nil nil)
          (let ((lexical-binding
                 (let ((cookie (save-current-buffer (set-buffer buf) (romacs--lexical-cookie))))
                   (cond ((eq cookie 'lex) t)
                         ((eq cookie 'dyn) nil)
                         ((and (boundp 'internal--get-default-lexical-binding-function)
                               internal--get-default-lexical-binding-function)
                          (funcall internal--get-default-lexical-binding-function buf))
                         (t (default-toplevel-value 'lexical-binding))))))
            (romacs--readevalloop buf filename printflag nil nil nil))))
      nil)))

(defun eval-region (start end &optional printflag read-function)
  "Execute the region as Lisp code.
When called from programs, expects two arguments, giving starting and
ending indices in the current buffer of the text to be executed.
PRINTFLAG nil discards the output of output functions in the code,
anything else is the stream to print to.  READ-FUNCTION, if non-nil,
reads each expression in place of `read'.  lread.c Feval_region over
`romacs--readevalloop'."
  (interactive "r")
  (let ((standard-output (or printflag 'symbolp))
        (eval-buffer-list (cons (current-buffer) eval-buffer-list)))
    (romacs--readevalloop (current-buffer) buffer-file-name printflag read-function start end)
    nil))

;; GNU fringe.c's bitmap registry.  A bitmap symbol's `fringe' property is
;; its number, as fringe.el gives the standard ones 1 to 24; a definition
;; keeps (ROWS WIDTH ALIGN PERIODIC) in `romacs--fringe-bitmap', ROWS
;; with each row's leftmost pixel in bit 15, which redisplay reads
;; (fringe.rs) before the standard bitmap of its number.

(defun define-fringe-bitmap (bitmap bits &optional height width align)
  "Define fringe bitmap BITMAP from BITS of size HEIGHT x WIDTH.
GNU fringe.c `Fdefine_fringe_bitmap'."
  (unless (symbolp bitmap) (signal 'wrong-type-argument (list 'symbolp bitmap)))
  (unless (or (vectorp bits) (stringp bits) (bool-vector-p bits))
    (signal 'wrong-type-argument (list 'arrayp bits)))
  (and height (not (integerp height)) (signal 'wrong-type-argument (list 'fixnump height)))
  (and width (not (integerp width)) (signal 'wrong-type-argument (list 'fixnump width)))
  (and width (or (< width 1) (> width 16))
       (signal 'args-out-of-range (list width "Width must be from 1 to 16")))
  (let* ((h (length bits))
         (rows (if height (max 0 (min height 255)) h))
         (fill (if (> rows h) (/ (- rows h) 2) 0))
         (w (or width 8))
         (periodic (and (consp align) (consp (cdr align)) (car (cdr align))))
         (align (if (consp align) (car align) align))
         (j rows)
         (bitmap-rows nil))
    (unless (memq align '(nil top center bottom)) (error "Bad align argument"))
    (while (> j 0)
      (setq j (1- j))
      (let* ((i (- j fill))
             (elt (and (>= i 0) (< i h) (aref bits i))))
        (setq bitmap-rows (cons (if (integerp elt) (logand (ash elt (- 16 w)) #xffff) 0) bitmap-rows))))
    (unless (get bitmap 'fringe)
      (let ((n 25) (tail fringe-bitmaps))
        ;; The least number above the standard ones no bitmap holds.
        (while tail
          (if (eq (get (car tail) 'fringe) n)
              (setq n (1+ n) tail fringe-bitmaps)
            (setq tail (cdr tail))))
        (setq fringe-bitmaps (cons bitmap fringe-bitmaps))
        (put bitmap 'fringe n)))
    (put bitmap 'romacs--fringe-bitmap (list bitmap-rows w align periodic))
    ;; Windows show the new bits on their next redisplay.
    (force-window-update)
    bitmap))

(defun destroy-fringe-bitmap (bitmap)
  "Destroy fringe bitmap BITMAP, restoring a standard one it overrode.
GNU fringe.c `Fdestroy_fringe_bitmap'."
  (unless (symbolp bitmap) (signal 'wrong-type-argument (list 'symbolp bitmap)))
  (let ((n (get bitmap 'fringe)))
    (when (integerp n)
      (put bitmap 'romacs--fringe-bitmap nil)
      (put bitmap 'romacs--fringe-face nil)
      (when (> n 24)
        (setq fringe-bitmaps (delq bitmap fringe-bitmaps))
        (put bitmap 'fringe nil))
      (force-window-update)))
  nil)

(defun set-fringe-bitmap-face (bitmap &optional face)
  "Set face for fringe bitmap BITMAP to FACE; nil restores the `fringe' face.
GNU fringe.c `Fset_fringe_bitmap_face'."
  (unless (integerp (get bitmap 'fringe)) (error "Undefined fringe bitmap"))
  (put bitmap 'romacs--fringe-face face)
  (force-window-update)
  nil)

;; GNU xdisp.c's tab bar.  Its row is the captions of the menu items the
;; [tab-bar] key binds in the active keymaps (tab_bar_items,
;; build_desired_tab_bar_string), which tab-bar.el's keymap filter makes,
;; and a click runs the binding of [tab-bar EVENT] with a position whose
;; column `tab-bar--event-to-item' maps back to an item over the same walk.

(defun romacs--tab-bar-items ()
  "The tab bar's menu items, (KEY CAPTION BINDING), in their order."
  (let ((map (lookup-key (cons 'keymap (nreverse (current-active-maps))) [tab-bar]))
        (items nil))
    (when (keymapp map)
      (map-keymap
       (lambda (key binding)
         (when (and (eq (car-safe binding) 'menu-item)
                    (let ((visible (plist-member (nthcdr 3 binding) :visible)))
                      (or (not visible) (eval (car (cdr visible)) t))))
           (let ((caption (nth 1 binding)))
             (setq caption (if (stringp caption) caption (eval caption t)))
             (when (stringp caption)
               (setq items (cons (list key caption (nth 2 binding)) items))))))
       map))
    (nreverse items)))

(defun romacs--tab-bar-string ()
  "The tab bar's row: its items' captions, concatenated."
  (apply #'concat (mapcar (lambda (item) (nth 1 item)) (romacs--tab-bar-items))))

(defun romacs--tab-bar-click (column type)
  "Run the tab bar's binding for a mouse event of TYPE at COLUMN of its row.
The event's position has no window, as on a text terminal, so the tab bar's
commands find the item by COLUMN; its key sequence is [tab-bar TYPE]."
  (let* ((event (list type (list nil 'tab-bar (cons column 0) 0)))
         (keys (vector 'tab-bar event))
         (command (key-binding (vector 'tab-bar type) t)))
    (when (commandp command)
      (setq last-input-event event
            last-nonmenu-event event
            last-command-event event
            this-command command)
      (call-interactively command nil keys)
      (setq last-command command))))

;;;; The mouse: frame.c, over what the host says of the pointer

(defun romacs--mouse (x y)
  "(FRAME X . Y) on the selected frame, through `mouse-position-function'."
  (let ((position (cons (selected-frame) (cons x y))))
    (if mouse-position-function (funcall mouse-position-function position) position)))

(defun mouse-pixel-position ()
  "Return a list (FRAME X . Y) giving the mouse frame and position in pixels.
X and Y are nil when the mouse is not over the frame, or there is none."
  (let ((at (romacs--pointer)))
    (romacs--mouse (car at) (cdr at))))

(defun mouse-position ()
  "Return a list (FRAME X . Y) giving the mouse frame and position in characters.
X and Y are nil when the mouse is not over the frame, or there is none."
  (let ((at (romacs--pointer)))
    (if at
        (let ((border (frame-internal-border-width)))
          (romacs--mouse (floor (- (car at) border) (frame-char-width))
                         (floor (- (cdr at) border) (frame-char-height))))
      (romacs--mouse nil nil))))

(defun mouse-position-in-root-frame ()
  "Return (X . Y), `mouse-position' on the root frame.
Romacs's frames measure from the root frame's corner, as a Wayland
frame's position is not known."
  (let ((position (mouse-position)))
    (and (framep (car position))
         (cons (or (car (cdr position)) 0) (or (cdr (cdr position)) 0)))))

(defun romacs--check-mouse-target (frame x y)
  "frame.c's checks of FRAME, X and Y, which nothing warps with.
pgtkterm.c frame_set_mouse_pixel_position is empty: Wayland has no warp."
  (unless (frame-live-p frame) (signal 'wrong-type-argument (list 'frame-live-p frame)))
  (dolist (n (list x y))
    (unless (integerp n) (signal 'wrong-type-argument (list 'integerp n)))
    (unless (<= -2147483648 n 2147483647)
      (signal 'args-out-of-range (list n -2147483648 2147483647)))))

(defun set-mouse-position (frame x y)
  "Ask to move the mouse pointer to character X, Y of FRAME.
Wayland lets no client do it, so nothing moves, as pgtk's."
  (romacs--check-mouse-target frame x y))

(defun set-mouse-pixel-position (frame x y)
  "Ask to move the mouse pointer to pixel X, Y of FRAME.
Wayland lets no client do it, so nothing moves, as pgtk's."
  (romacs--check-mouse-target frame x y))

;;;; The window title

(defun romacs--title-format-static-p (format)
  "Whether the mode-line FORMAT reads only `%b', `%%', the symbols of GNU's
default, and fixed text: its expansion moves only with what
`Vm::frame_title_key' covers."
  (cond ((stringp format) (not (string-match-p "%[^b%]" format)))
        ((consp format) (and (not (keywordp (car format)))
                             (romacs--title-format-static-p (car format))
                             (romacs--title-format-static-p (cdr format))))
        ((integerp format) t)
        (t (memq format '(nil t multiple-frames system-name)))))

;;;; Word motion through `find-word-boundary-function-table'

(defun romacs--word-at-p (position)
  "Whether the character at POSITION is a word's, as syntax.c scan_words reads it."
  (let ((class (logand (car (syntax-after position)) 65535)))
    (or (= class 2) (and words-include-escapes (memq class '(9 10))))))

(defun romacs--scan-words (from count)
  "syntax.c scan_words: the position COUNT words from FROM, else nil.
Nil means an edge of the accessible text came first.  A word start or
end whose character has a function in `find-word-boundary-function-table'
goes where that says."
  (let ((beg (point-min)) (end (point-max)) (table find-word-boundary-function-table))
    (catch 'edge
      (while (> count 0)
        (while (progn (when (= from end) (throw 'edge nil))
                      (prog1 (not (romacs--word-at-p from)) (setq from (1+ from)))))
        (let ((function (aref table (char-after (1- from)))))
          (if (fboundp function)
              (let ((position (funcall function (1- from) end)))
                (when (and (fixnump position) (< from position) (<= position (point-max)))
                  (setq from position)))
            (while (and (/= from end) (romacs--word-at-p from)) (setq from (1+ from)))))
        (setq count (1- count)))
      (while (< count 0)
        (while (progn (when (= from beg) (throw 'edge nil))
                      (setq from (1- from))
                      (not (romacs--word-at-p from))))
        (let ((function (aref table (char-after from))))
          (if (fboundp function)
              (let ((position (funcall function from beg)))
                (when (and (fixnump position) (<= (point-min) position) (< position from))
                  (setq from position)))
            (while (and (/= from beg) (romacs--word-at-p (1- from))) (setq from (1- from)))))
        (setq count (1+ count)))
      from)))

(defun romacs--forward-word (count)
  "syntax.c Fforward_word over `romacs--scan-words'."
  (let* ((reached (romacs--scan-words (point) count))
         (position (or reached (if (> count 0) (point-max) (point-min))))
         (kept (constrain-to-field position (point))))
    (goto-char kept)
    (eq kept reached)))

;; lread.c substitute_object_recurse, for edebug.el's reader of `#N=': PLACEHOLDER
;; replaced by OBJECT everywhere in OBJECT.  With COMPLETED t every node is
;; remembered; a hash table names the nodes that can start a cycle.
(defun lread--substitute-object-in-subtree (object placeholder completed)
  "Replace PLACEHOLDER with OBJECT in OBJECT, as the reader does for `#N='.
COMPLETED is t, or a hash table of the nodes that may be cycle entry points."
  (let ((seen (make-hash-table :test 'eq)) (absent (list nil)) recurse)
    (setq recurse
          (lambda (tree)
            (cond
             ((eq tree placeholder) object)
             ((or (symbolp tree) (numberp tree) (bool-vector-p tree) (gethash tree seen)) tree)
             (t
              (when (or (eq completed t) (not (eq (gethash tree completed absent) absent)))
                (puthash tree t seen))
              (cond
               ((consp tree)
                (let ((cell tree) (going t))
                  (while going
                    (setcar cell (funcall recurse (car cell)))
                    (let ((next (cdr cell)))
                      (if (and (consp next) (not (eq next placeholder)) (not (gethash next seen)))
                          (progn
                            (when (or (eq completed t) (not (eq (gethash next completed absent) absent)))
                              (puthash next t seen))
                            (setq cell next))
                        (setcdr cell (funcall recurse next))
                        (setq going nil))))))
               ((or (vectorp tree) (recordp tree))
                (dotimes (i (length tree))
                  (aset tree i (funcall recurse (aref tree i)))))
               ((hash-table-p tree)
                (maphash (lambda (key value) (puthash key (funcall recurse value) tree)) tree))
               ((stringp tree)
                (dolist (interval (object-intervals tree))
                  (set-text-properties (nth 0 interval) (nth 1 interval)
                                       (funcall recurse (nth 2 interval)) tree)))
               ((or (char-table-p tree) (closurep tree)))
               (t (signal 'wrong-type-argument (list 'sequencep tree))))
              tree))))
    (unless (eq (funcall recurse object) object)
      (error "Unexpected mutation error in reader"))
    nil))

;;; prelude.el ends here
