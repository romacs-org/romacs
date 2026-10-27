;;; prelude.el --- VM2 T3 tier: loops that call Lisp  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; A Rust primitive never calls Lisp from a loop (PRIMITIVE-DOCTRINE section 3).
;; A loop whose body calls Lisp is written here instead, as ordinary compiled
;; Lisp over leaf cursors (`car'/`cdr', `aref', `length'), the same shape as the
;; `maphash' expansion over `#:vm2-hash-next' in expand.rs.  Every value such a
;; loop keeps across a callback is then a register of a traced Lisp frame by
;; construction, and a collection inside the callback forwards it.

;;; Code:

(defun mapcar (function sequence)
  "Apply FUNCTION to each element of SEQUENCE, and make a list of the results.
GNU fns.c `Fmapcar'/`mapcar1': the length is taken first, so an improper or
circular SEQUENCE signals before FUNCTION runs, and a list that FUNCTION
shortens ends the walk early."
  (let ((n (length sequence))
        (i 0)
        (acc nil))
    (if (or (consp sequence) (null sequence))
        (let ((tail sequence))
          (while (and (< i n) (consp tail))
            (setq acc (cons (funcall function (car tail)) acc))
            (setq tail (cdr tail))
            (setq i (1+ i))))
      (while (< i n)
        (setq acc (cons (funcall function (aref sequence i)) acc))
        (setq i (1+ i))))
    (nreverse acc)))


;;; Reverse keymap lookup

(defvar where-is-preferred-modifier nil)
(defvar minor-mode-overriding-map-alist nil)
(defvar emulation-mode-map-alists nil)

(defun romacs--where-definition (binding)
  (let ((seen nil))
    (while (and (consp binding)
                (or (eq (car binding) 'menu-item) (stringp (car binding))))
      (if (memq binding seen) (error "Circular menu item"))
      (setq seen (cons binding seen))
      (if (stringp (car binding))
          (setq binding (cdr binding))
        (if (not (consp (cdr binding)))
            (error "Malformed menu item")
          (setq binding (cdr (cdr binding)))
          (if (consp binding) (setq binding (car binding))))))
    binding))

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

(defun romacs--where-cycle-p (map prefix queue)
  (let ((cycle nil))
    (while (and queue (not cycle))
      (let* ((node (car queue)) (previous (car node)) (i 0))
        (if (and (eq map (car (cdr node)))
                 (<= (length previous) (length prefix)))
            (progn
              (while (and (< i (length previous))
                          (eq (aref previous i) (aref prefix i)))
                (setq i (1+ i)))
              (setq cycle (= i (length previous))))))
      (setq queue (cdr queue)))
    cycle))

(defun romacs--where-bindings (maps)
  (let ((rows nil))
    (while (consp maps)
      (let* ((root (lookup-key (car maps) []))
             (queue (list (list [] root)))
             (last queue)
             (tail queue))
        (while tail
          (let* ((node (car tail)) (prefix (car node))
                 (map (car (cdr node))))
            (map-keymap
             (lambda (event raw)
               (let* ((sequence (romacs--where-append prefix event))
                      (binding (romacs--where-definition raw)))
                 (setq rows (cons (cons sequence raw) rows))
                 (if (keymapp binding)
                     (let ((child (lookup-key binding [])))
                       (if (not (romacs--where-cycle-p child prefix queue))
                           (let ((entry (list sequence child)))
                             ;; Meta prefixes do not increase sequence length.
                             (if (= (length sequence) (length prefix))
                                 (progn
                                   (setcdr tail (cons entry (cdr tail)))
                                   (if (eq last tail) (setq last (cdr tail))))
                               (setcdr last (list entry))
                               (setq last (cdr last)))))))))
             map))
          (setq tail (cdr tail))))
      (setq maps (cdr maps)))
    (nreverse rows)))

(defun romacs--where-active-maps ()
  (let* ((local (romacs--keymap-at-point 'local-map))
         (text (romacs--keymap-at-point 'keymap))
         (maps (list (current-global-map)))
         (override (if (boundp 'minor-mode-overriding-map-alist)
                       minor-mode-overriding-map-alist))
         (emulation (if (boundp 'emulation-mode-map-alists)
                        emulation-mode-map-alists))
         (alists (list override minor-mode-map-alist))
         (minor nil))
    (if (not (keymapp local)) (setq local (current-local-map)))
    (if local (setq maps (cons local maps)))
    (while emulation
      (let ((alist (car emulation)))
        (if (symbolp alist)
            (setq alist (if (boundp alist) (symbol-value alist))))
        (setq minor (append minor (romacs--where-minor-maps alist nil))))
      (setq emulation (cdr emulation)))
    (while alists
      (setq minor (append minor (romacs--where-minor-maps
                                (car alists) (if (cdr alists) nil override))))
      (setq alists (cdr alists)))
    (setq maps (append minor maps))
    (if (keymapp text) (setq maps (cons text maps)))
    maps))

(defun romacs--where-minor-maps (alist ignored)
  (let ((maps nil))
    (while (consp alist)
      (let ((entry (car alist)))
        (if (and (consp entry) (symbolp (car entry))
                 (boundp (car entry)) (symbol-value (car entry))
                 (not (assq (car entry) ignored)))
            (let ((map (indirect-function (cdr entry))))
              (if map (setq maps (cons map maps))))))
      (setq alist (cdr alist)))
    (nreverse maps)))

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
        (if (not noindirect) (setq binding (romacs--where-definition binding)))
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

(defun where-is-internal (definition &optional keymap firstonly noindirect no-remap)
  "Return keys invoking DEFINITION in KEYMAP, respecting shadows and remapping."
  (let* ((maps (cond ((and (consp keymap) (keymapp (car keymap))) keymap)
                     (keymap (list keymap (current-global-map)))
                     (t (romacs--where-active-maps))))
         (remapping (romacs--where-remapping definition maps))
         (modifier (romacs--where-modifier)))
    (if (and (not no-remap) remapping) (setq definition remapping))
    (or (and firstonly (romacs--where-advertised definition maps))
        (romacs--where-find definition maps firstonly noindirect no-remap modifier))))

(defun romacs--where-find (definition maps firstonly noindirect no-remap modifier)
  (let* ((rows (romacs--where-bindings maps))
         (nomenus (and firstonly (not (eq firstonly 'non-ascii))))
         (pending (romacs--where-candidates definition rows noindirect nomenus))
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
                    (setq remapped (append (romacs--where-candidates
                                            (aref sequence 1) rows noindirect nomenus)
                                           remapped))
                  (if (stringp (aref sequence (1- (length sequence))))
                      (aset sequence (1- (length sequence)) "(any string)"))
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

(defun previous-single-char-property-change (position prop &optional object limit)
  "Return the position of previous text property or overlay change for PROP.
textprop.c Fprevious_single_char_property_change."
  (if (stringp object)
      (or (previous-single-property-change position prop object limit)
          (if (null limit) 0
            (if (integerp limit) limit
              (signal 'wrong-type-argument (list 'fixnump limit)))))
    (if (and object (not (bufferp object)))
        (signal 'wrong-type-argument (list 'bufferp object)))
    (save-current-buffer
      (if object (set-buffer object))
      (setq position (romacs--fixnum-coerce-marker position))
      (setq limit (if limit (romacs--fixnum-coerce-marker limit) (point-min)))
      (if (<= position limit)
          (max limit (point-min))
        (let ((initial (get-char-property (- position (if (>= position 0) 1 0)) prop object)))
          (catch 'romacs--found
            (while t
              (setq position (previous-char-property-change position limit))
              (if (<= position limit)
                  (throw 'romacs--found limit)
                (unless (eq (get-char-property (1- position) prop object) initial)
                  (throw 'romacs--found position))))))))))

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
  (let ((inhibit-redisplay t))
    (condition-case err (apply function args)
      (t (let ((inhibit-message t))
           (message "%s" (format "Error muted by safe_call: %S signaled %S" (cons function args) err)))
         nil))))

;;; prelude.el ends here
