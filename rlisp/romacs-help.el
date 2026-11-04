;;; romacs-help.el --- GNU's help primitives in Lisp -*- lexical-binding: t; -*-
;; Copyright (C) 1985-1988, 1993-1995, 1998-2026 Free Software
;; Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The primitives help.el and help-fns.el call to describe keymaps,
;; char-tables and syntax: `describe-buffer-bindings' (C-h b, C-h m),
;; `describe-vector', `help--describe-vector' (a keymap's vectors and
;; char-tables), `text-char-description' and
;; `internal-describe-syntax-value' (describe-syntax).  GNU writes them in
;; C; Romacs in Lisp, over the keymap layer of romacs-keymap.el, keeping
;; GNU's output byte for byte.
;;
;; Adapted from GNU src/keymap.c (`describe-buffer-bindings',
;; `describe_vector', `describe-vector', `help--describe-vector',
;; `text-char-description') and src/syntax.c
;; (`internal-describe-syntax-value').

;;; Code:

(defvar describe-bindings-check-shadowing-in-ranges nil
  "If non-nil, consider command shadowing when describing ranges of keys.
If the value is t, describing bindings of consecutive keys will not
report them as a single range if they are shadowed by different
minor-mode commands.
If the value is `ignore-self-insert', assume that consecutive keys
bound to `self-insert-command' are not all shadowed; this speeds up
commands such as \\[describe-bindings] and \\[describe-mode], but could miss some shadowing.
Any other non-nil value is treated is t.

Beware: setting this non-nil could potentially slow down commands
that describe key bindings.  That is why the default is nil.")

(defconst romacs-help--8-bit-start #x3FFF80
  "The first raw 8-bit character: a char-table's ranges stop before it
\(keymap.c describe_vector, MAX_5_BYTE_CHAR + 1).")

;;;; Keys and characters

(defun text-char-description (character)
  "Return the description of CHARACTER in standard Emacs notation.
CHARACTER must be a valid character code that passes the `characterp' test.
Control characters turn into \"^char\", and characters with Meta and other
modifiers signal an error, as they are not valid character codes.
This differs from `single-key-description' which accepts character events,
and thus doesn't enforce the `characterp' condition, turns control
characters into \"C-char\", and uses the 2**27 bit for Meta.
See Info node `(elisp)Describing Characters' for examples."
  (unless (characterp character) (signal 'wrong-type-argument (list 'characterp character)))
  (cond ((> character 127) (string character))
        ((< character 32) (string ?^ (+ character 64)))
        ((= character 127) (string ?^ ??))
        (t (string character))))

(defun romacs-help--insertable (string)
  "STRING with each character above #x10FFFF that is not a raw byte as
U+FFFD, one column as GNU counts it, so a section aligns as GNU's: a
Romacs buffer cannot hold one (DIVERGENCES.md VM2-HELP-EXTENDED)."
  (if (not (seq-some (lambda (c) (< #x10FFFF c #x3FFF80)) string))
      string
    (mapconcat (lambda (c) (string (if (< #x10FFFF c #x3FFF80) #xFFFD c))) string "")))

(defun romacs-help--key (key prefix keymap-p)
  "KEY's description after PREFIX, in the key-binding face in a keymap
\(describe_key_maybe_fontify)."
  (let ((description (romacs-help--insertable (key-description key prefix))))
    (when keymap-p
      (add-text-properties 0 (length description) '(font-lock-face help-key-binding) description))
    description))

(defun romacs-help--shadow-lookup (shadow key)
  "KEY's binding in the SHADOW keymaps, defaults included; nil when KEY
is too long for them (keymap.c shadow_lookup)."
  (let ((value (lookup-key shadow key t)))
    (unless (natnump value) value)))

;;;; Vectors and char-tables

(defun romacs-help--check-vector (vector)
  (unless (or (vectorp vector) (char-table-p vector))
    (signal 'wrong-type-argument (list 'vector-or-char-table-p vector))))

(defun romacs-help--char-table-ranges (table)
  "TABLE's values as ascending (FROM TO VALUE), each the widest run of
`eq' values, TABLE's own slots only (no parent), a slot holding nil
taking the default, and no run crossing into the raw 8-bit characters
\(chartab.c char_table_ref_and_range)."
  (let ((parent (char-table-parent table)) (default (char-table-range table nil)) (runs nil) (merged nil))
    (unwind-protect
        (progn
          (set-char-table-parent table nil)
          (map-char-table (lambda (key value)
                            (push (if (consp key) (list (car key) (cdr key) value) (list key key value)) runs))
                          table))
      (set-char-table-parent table parent))
    (setq runs (sort runs (lambda (a b) (< (car a) (car b)))))
    ;; A gap holds nil, which reads as the default.
    (let ((next 0))
      (dolist (run (append runs (list (list (1+ (max-char)) (1+ (max-char)) nil))))
        (when (and default (< next (car run)))
          (push (list next (1- (car run)) default) merged))
        (when (<= (car run) (max-char)) (push run merged))
        (setq next (1+ (nth 1 run)))))
    (setq merged (nreverse merged))
    ;; Widest runs of one value, split before the raw 8-bit characters.
    (let ((result nil))
      (dolist (run merged)
        (let ((last (car result)))
          (if (and last (eq (nth 2 last) (nth 2 run)) (= (1+ (nth 1 last)) (car run))
                   (not (= (car run) romacs-help--8-bit-start)))
              (setcar (cdr last) (nth 1 run))
            (push (copy-sequence run) result))))
      (let ((split nil))
        (dolist (run (nreverse result))
          (if (and (< (car run) romacs-help--8-bit-start) (>= (nth 1 run) romacs-help--8-bit-start))
              (progn (push (list (car run) (1- romacs-help--8-bit-start) (nth 2 run)) split)
                     (push (list romacs-help--8-bit-start (nth 1 run) (nth 2 run)) split))
            (push run split)))
        (nreverse split)))))

(defun romacs-help--describe-entry (state from definition last)
  "Describe DEFINITION, bound from FROM in the vector STATE describes, as
one entry; return the last index it covers, LAST at most
\(describe_vector's loop body)."
  (pcase-let* ((`(,vector ,prefix ,describer ,element ,partial ,shadow ,entire-map ,keymap-p ,mention-shadow ,elt-prefix ,first) state)
              ;; A char-table's run is one entry up to its end; a vector's
              ;; slots join below.
              (i (if (char-table-p vector) (1- last) from)) (shadowed-by nil) (this-shadowed nil))
    (catch 'skip
      (when (and partial (symbolp definition) (get definition 'suppress-keymap)) (throw 'skip nil))
      (let ((key (vector from)))
        (when shadow
          (setq shadowed-by (romacs-help--shadow-lookup shadow key))
          (when (and shadowed-by (not (eq shadowed-by definition)))
            (if mention-shadow (setq this-shadowed t) (throw 'skip nil))))
        (when (and entire-map (not (eq (lookup-key entire-map key t) definition))) (throw 'skip nil))
        (when (car first) (insert "\n") (setcar first nil))
        (when elt-prefix (insert elt-prefix))
        (insert (romacs-help--key key prefix keymap-p))
        ;; Consecutive vector slots with an `equal' definition are one range.
        (unless (char-table-p vector)
          (let (next)
            (while (and (< (1+ i) last)
                        (setq next (keymap--get-keyelt (aref vector (1+ i)) nil))
                        (equal next definition))
              (setq i (1+ i)))))
        (when (and describe-bindings-check-shadowing-in-ranges (char-table-p vector) (/= i from)
                   (or (not (eq describe-bindings-check-shadowing-in-ranges 'ignore-self-insert))
                       (not (eq definition 'self-insert-command))))
          ;; The range ends before its first key shadowed otherwise.
          (let ((j (1+ from)))
            (while (<= j i)
              (unless (equal (romacs-help--shadow-lookup shadow (vector j)) shadowed-by)
                (setq i (1- j)))
              (setq j (1+ j)))))
        (when (/= i from)
          (insert " .. ")
          (when elt-prefix (insert elt-prefix))
          (insert (romacs-help--key (vector i) prefix keymap-p)))
        (funcall element definition describer)
        (when this-shadowed
          (backward-char 1)
          (insert (if (symbolp shadowed-by)
                      (format-message "  (currently shadowed by `%s')" shadowed-by)
                    "  (currently shadowed)"))
          (forward-char 1))))
    i))

(defun romacs-help--describe-vector (vector prefix describer element partial shadow entire-map keymap-p mention-shadow)
  "Insert a description of VECTOR's contents, calling ELEMENT with each
definition and DESCRIBER (keymap.c describe_vector)."
  (let* ((elt-prefix (and (not keymap-p) prefix (> (length prefix) 0)
                          (concat (key-description prefix) " ")))
         (prefix (and keymap-p prefix))
         (state (list vector prefix describer element partial shadow entire-map keymap-p mention-shadow
                      elt-prefix (list t))))
    (if (char-table-p vector)
        (dolist (run (romacs-help--char-table-ranges vector))
          (let ((from (car run)) (to (nth 1 run)) (definition (keymap--get-keyelt (nth 2 run) nil)))
            (when definition
              (while (<= from to)
                (setq from (1+ (romacs-help--describe-entry state from definition (1+ to))))))))
      (let ((i 0) (size (length vector)))
        (while (< i size)
          (let ((definition (keymap--get-keyelt (aref vector i) nil)))
            (setq i (1+ (if definition (romacs-help--describe-entry state i definition size) i)))))))
    (when (and (char-table-p vector) (char-table-range vector nil))
      (when elt-prefix (insert elt-prefix))
      (insert "default")
      (funcall element (char-table-range vector nil) describer))))

(defun describe-vector (vector &optional describer)
  "Insert a description of contents of VECTOR.
This is text showing the elements of vector matched against indices.
DESCRIBER is the output function used; nil means use `princ'."
  (let ((standard-output (current-buffer)))
    (romacs-help--check-vector vector)
    (romacs-help--describe-vector vector nil (or describer #'princ)
                                  (lambda (definition describer)
                                    (indent-to 16 1)
                                    (funcall describer definition)
                                    (terpri))
                                  nil nil nil nil nil)
    nil))

(defun help--describe-vector (vector prefix describer partial shadow entire-map mention-shadow)
  "Insert in the current buffer a description of the contents of VECTOR.
Call DESCRIBER to insert the description of one value found in VECTOR.

PREFIX is a string describing the key which leads to the keymap that
this vector is in.

If PARTIAL, it means do not mention suppressed commands.

SHADOW is a list of keymaps that shadow this map.
If it is non-nil, look up the key in those maps and don't mention it
if it is defined by any of them.

ENTIRE-MAP is the keymap in which this vector appears.
If the definition in effect in the whole map does not match
the one in this keymap, we ignore this one."
  (let ((standard-output (current-buffer)))
    (romacs-help--check-vector vector)
    (romacs-help--describe-vector vector prefix describer
                                  (lambda (definition describer) (funcall describer definition))
                                  partial shadow entire-map t mention-shadow)
    nil))

;;;; All of a buffer's bindings

(defun describe-buffer-bindings (buffer &optional prefix menus)
  "Insert the list of all defined keys and their definitions.
The list is inserted in the current buffer, while the bindings are
looked up in BUFFER.
The optional argument PREFIX, if non-nil, should be a key sequence;
then we display only bindings that start with that prefix.
The optional argument MENUS, if non-nil, says to mention menu bindings.
\(Ordinarily these are omitted from the output.)"
  (unless (bufferp buffer) (signal 'wrong-type-argument (list 'bufferp buffer)))
  (let* ((nomenu (not menus)) (shadow nil)
         (tree (lambda (map partial shadow title transl always-title)
                (help--describe-map-tree map partial shadow prefix title nomenu transl always-title nil buffer))))
    ;; Report on alternates for keys.
    (when (and (stringp keyboard-translate-table) prefix)
      (let ((heading "Keyboard translations:\n\nYou type        Translation\n--------        -----------\n"))
        (dotimes (c (length keyboard-translate-table))
          (unless (= (aref keyboard-translate-table c) c)
            (when heading (insert heading) (setq heading nil))
            (insert (single-key-description (aref keyboard-translate-table c)))
            (indent-to 16 1)
            (insert (single-key-description c) "\n")))
        (insert "\n")))
    (when key-translation-map
      (funcall tree key-translation-map nil nil "Key translations" t nil))
    (let ((start overriding-terminal-local-map))
      (if start
          (progn (funcall tree start t shadow "\f\nOverriding Bindings" nil nil)
                 (push start shadow)
                 (setq start nil))
        (setq start overriding-local-map))
      (if start
          (progn (funcall tree start t shadow "\f\nOverriding Bindings" nil nil)
                 (push start shadow))
        ;; The minor and major mode maps, as BUFFER has them.
        (pcase-let ((`(,minor ,keymap ,local-map ,own-map ,mode)
                     (with-current-buffer buffer
                       (list (romacs-keymap--minor-mode-maps)
                             (romacs-keymap--local-map (point) 'keymap)
                             (romacs-keymap--local-map (point) 'local-map)
                             (current-local-map) major-mode))))
          (when keymap
            (funcall tree keymap t shadow "\f\n`keymap' Property Bindings" nil nil)
            (push keymap shadow))
          (dolist (entry minor)
            (funcall tree (cdr entry) t shadow (format "\f\n`%s' Minor Mode Bindings" (car entry)) nil nil)
            (push (cdr entry) shadow))
          (when local-map
            (funcall tree local-map t shadow
                     (if (eq local-map own-map)
                         (format "\f\n`%s' Major Mode Bindings" mode)
                       "\f\n`local-map' Property Bindings")
                     nil nil)
            (push local-map shadow)))))
    (funcall tree (current-global-map) t shadow "\f\nGlobal Bindings" nil t)
    (when local-function-key-map
      (funcall tree local-function-key-map nil nil "\f\nFunction key map translations" t nil))
    (when input-decode-map
      (funcall tree input-decode-map nil nil "\f\nInput decoding map translations" t nil))
    nil))

;;;; Syntax

(defconst romacs-help--syntax-classes
  ["whitespace" "punctuation" "word" "symbol" "open" "close" "prefix" "string"
   "math" "escape" "charquote" "comment" "endcomment" "inherit" "comment fence" "string fence"]
  "Each syntax class's name, by class code (syntax.c).")

(defun internal-describe-syntax-value (syntax)
  "Insert a description of the internal syntax description SYNTAX at point."
  (cond
   ((null syntax) (insert "default"))
   ((char-table-p syntax) (insert "deeper char-table ..."))
   ((or (not (consp syntax)) (not (fixnump (car syntax)))
        (not (or (null (cdr syntax)) (characterp (cdr syntax)))))
    (insert "invalid"))
   (t
    (let* ((flags (logand (car syntax) #x7fffffff))
           (code (logand flags #xff))
           (match (cdr syntax))
           (bit (lambda (n) (= 1 (logand (ash flags (- n)) 1))))
           (start1 (funcall bit 16)) (start2 (funcall bit 17))
           (end1 (funcall bit 18)) (end2 (funcall bit 19))
           (prefix (funcall bit 20)) (comstyleb (funcall bit 21))
           (comstylec (funcall bit 23)) (comnested (funcall bit 22)))
      (if (>= code (length romacs-help--syntax-classes))
          (insert "invalid")
        (insert (aref " .w_()'\"$\\/<>@!|" code))
        (if match (insert match) (insert " "))
        (when start1 (insert "1"))
        (when start2 (insert "2"))
        (when end1 (insert "3"))
        (when end2 (insert "4"))
        (when prefix (insert "p"))
        (when comstyleb (insert "b"))
        (when comstylec (insert "c"))
        (when comnested (insert "n"))
        (insert "\twhich means: " (aref romacs-help--syntax-classes code))
        (when match (insert ", matches " match))
        (when start1 (insert ",\n\t  is the first character of a comment-start sequence"))
        (when start2 (insert ",\n\t  is the second character of a comment-start sequence"))
        (when end1 (insert ",\n\t  is the first character of a comment-end sequence"))
        (when end2 (insert ",\n\t  is the second character of a comment-end sequence"))
        (when comstyleb (insert " (comment style b)"))
        (when comstylec (insert " (comment style c)"))
        (when comnested (insert " (nestable)"))
        (when prefix
          (insert (substitute-command-keys
                   ",\n\t  is a prefix character for `backward-prefix-chars'")))))))
  syntax)

(provide 'romacs-help)
;;; romacs-help.el ends here
