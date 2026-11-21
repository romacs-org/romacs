;;; romacs-live-forms.el --- A Lisp file as definitions, for live updates  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The pure half of `romacs-live-update' (romacs-live.el): a file's text
;; read as top-level forms, each keyed by what it defines, and two texts
;; compared definition by definition.  What the update does about each
;; changed definition (a keymap replaced in place, an option given its new
;; default, a struct whose layout changed refused) is decided from these
;; values; nothing here touches the session.  A worker VM loads this file
;; too, to flatten a keymap it computed (`romacs-live-forms-keymap-flat').

;;; Code:

(require 'cl-lib)
(require 'seq)

(define-error 'romacs-live-unreadable "Live update: a file does not read")

(defun romacs-live-forms-read (text &optional name)
  "The top-level forms of TEXT, Lisp source, in order.
Signal `romacs-live-unreadable' with NAME and the position when a form
does not read."
  (with-temp-buffer
    (set-syntax-table emacs-lisp-mode-syntax-table)
    (insert text)
    (goto-char (point-min))
    (let ((forms nil))
      (condition-case err
          (while (progn (forward-comment (buffer-size)) (not (eobp)))
            (push (read (current-buffer)) forms))
        (error (signal 'romacs-live-unreadable (list name (point) (error-message-string err)))))
      (nreverse forms))))

(defconst romacs-live-forms--kinds
  '((defun . function) (defmacro . macro) (defsubst . macro) (define-inline . macro)
    (cl-defun . function) (cl-defmacro . macro) (cl-defsubst . macro) (cl-defgeneric . function)
    (defalias . function) (defvar . variable) (defconst . constant) (defvar-local . variable)
    (defvar-keymap . keymap) (defcustom . option) (defface . face) (cl-defstruct . struct)
    (defclass . class) (cl-defmethod . method) (define-derived-mode . mode)
    (define-minor-mode . mode) (define-globalized-minor-mode . mode))
  "Definers by the kind of definition they make.  A `macro' kind changes
what code compiled against it does: the files that require its library
load again after it.")

(defun romacs-live-forms--name (form)
  "The name FORM's definer gives, or nil: a symbol, quoted for `defalias'."
  (let ((name (nth 1 form)))
    (cond ((and (eq (car form) 'defalias) (eq (car-safe name) 'quote)) (cadr name))
          ((eq (car form) 'defalias) nil)
          ((eq (car form) 'cl-defstruct) (if (consp name) (car name) name))
          ((symbolp name) name))))

(defun romacs-live-forms--qualifiers (form)
  "What tells one `cl-defmethod' FORM of a name from another: its
qualifiers and argument list's specializers."
  (let ((rest (nthcdr 2 form)) (qualifiers nil))
    (while (and rest (not (listp (car rest))))
      (push (pop rest) qualifiers))
    (list (nreverse qualifiers)
          (mapcar (lambda (arg) (if (consp arg) (cadr arg) t))
                  (seq-take-while (lambda (arg) (not (memq arg '(&optional &rest &key &context))))
                                  (car rest))))))

(defun romacs-live-forms-key (form)
  "What top-level FORM defines, as (KIND NAME . DETAIL); nil for a form
that defines nothing by name."
  (let* ((kind (and (consp form) (cdr (assq (car form) romacs-live-forms--kinds))))
         (name (and kind (romacs-live-forms--name form))))
    (when name
      (if (eq kind 'method)
          (cons kind (cons name (romacs-live-forms--qualifiers form)))
        (list kind name)))))

(defvar romacs-live-forms--declared nil
  "Definers declared by the forms being compared, an alist of MACRO to
the definer it is read as: a source that declares one is read so before
the session has loaded the declaration.")

(defun romacs-live-forms-declarations (forms)
  "The definers FORMS declare at top level, an alist of MACRO to DEFINER:
each \=(put \='MACRO \='romacs-live-definer \='DEFINER)."
  (cl-loop for form in forms
           when (and (eq (car-safe form) 'put)
                     (equal (nth 2 form) '(quote romacs-live-definer))
                     (romacs-live-forms--quoted (nth 1 form)) (romacs-live-forms--quoted (nth 3 form)))
           collect (cons (romacs-live-forms--quoted (nth 1 form)) (romacs-live-forms--quoted (nth 3 form)))))

(defun romacs-live-forms--canonical (form)
  "FORM with its definer the one its `romacs-live-definer' property names:
a macro wrapping `cl-defstruct' (say) declares
\=(put \='MACRO \='romacs-live-definer \='cl-defstruct), and its forms are
read as that definer's.  `romacs-live-forms--declared' comes first."
  (let ((definer (and (symbolp (car-safe form))
                      (or (alist-get (car form) romacs-live-forms--declared)
                          (get (car form) 'romacs-live-definer)))))
    (if definer (cons definer (cdr form)) form)))

(defun romacs-live-forms-flatten (forms)
  "FORMS with the bodies of top-level `progn', `eval-and-compile' and
`eval-when-compile' in their place, as `load' runs them, each read as its
declared definer's (`romacs-live-forms--canonical')."
  (cl-loop for form in forms
           if (memq (car-safe form) '(progn eval-and-compile eval-when-compile))
           append (romacs-live-forms-flatten (cdr form))
           else collect (romacs-live-forms--canonical form)))

(defun romacs-live-forms-definitions (forms)
  "FORMS (flattened) as (DEFINITIONS . OTHERS): DEFINITIONS an alist of
keys (`romacs-live-forms-key') to their last form, in order; OTHERS the
forms that define nothing, each keyed (other INDEX . FORM-HASH) by its
place among them."
  (let ((definitions nil) (others nil) (index 0))
    (dolist (form (romacs-live-forms-flatten forms))
      (if-let* ((key (romacs-live-forms-key form)))
          (setf (alist-get key definitions nil nil #'equal) form)
        (push (cons (list 'other (cl-incf index)) form) others)))
    (cons (nreverse definitions) (nreverse others))))

(defun romacs-live-forms-diff (old new)
  "How the forms NEW differ from OLD, definition by definition: a plist of
`:added', `:changed' and `:removed' keys, `:others' (the number of forms
that define nothing and changed, are new or are gone) and `:unchanged'
\(the number of definitions alike in both)."
  (pcase-let ((`(,old-defs . ,old-others) (romacs-live-forms-definitions old))
              (`(,new-defs . ,new-others) (romacs-live-forms-definitions new))
              (added nil) (changed nil) (removed nil) (unchanged 0))
    (pcase-dolist (`(,key . ,form) new-defs)
      (let ((before (assoc key old-defs)))
        (cond ((not before) (push key added))
              ((equal (cdr before) form) (cl-incf unchanged))
              (t (push key changed)))))
    (pcase-dolist (`(,key . ,_) old-defs)
      (unless (assoc key new-defs) (push key removed)))
    (list :added (nreverse added) :changed (nreverse changed) :removed (nreverse removed)
          :others (+ (cl-count-if-not (lambda (form) (member form (mapcar #'cdr old-others))) (mapcar #'cdr new-others))
                     (cl-count-if-not (lambda (form) (member form (mapcar #'cdr new-others))) (mapcar #'cdr old-others)))
          :unchanged unchanged)))

(defun romacs-live-forms-macro-change-p (diff)
  "Whether DIFF changes a definition compiled code depends on: a macro,
an inline function or a struct, changed, gone or new (a function that
became a macro is one)."
  (cl-some (lambda (key) (memq (car key) '(macro struct)))
           (append (plist-get diff :changed) (plist-get diff :removed) (plist-get diff :added))))

(defun romacs-live-forms--quoted (form)
  "The symbol FORM quotes ('X or #'X), or nil."
  (and (memq (car-safe form) '(quote function)) (symbolp (cadr form)) (cadr form)))

(defun romacs-live-forms-requires (forms)
  "The features FORMS require at top level, in order."
  (delete-dups (cl-loop for form in (romacs-live-forms-flatten forms)
                        when (and (eq (car-safe form) 'require) (romacs-live-forms--quoted (nth 1 form)))
                        collect it)))

(defun romacs-live-forms-provides (forms)
  "The features FORMS provide at top level."
  (cl-loop for form in (romacs-live-forms-flatten forms)
           when (and (eq (car-safe form) 'provide) (romacs-live-forms--quoted (nth 1 form)))
           collect it))

(defun romacs-live-forms--slot-name (slot)
  (if (consp slot) (car slot) slot))

(defun romacs-live-forms-layout (form)
  "The layout a `cl-defstruct' or `defclass' FORM gives its records, as
\(PARENTS SLOTS REPRESENTATION): a struct's slot names and its `:type',
`:named' and `:initial-offset', a class's slots with their `:allocation';
nil for any other form."
  (pcase (car-safe form)
    ('cl-defstruct
     (let* ((spec (nth 1 form))
            (options (and (consp spec) (cdr spec)))
            (slots (nthcdr 2 form)))
       (when (stringp (car slots)) (setq slots (cdr slots)))
       (list (cl-loop for option in options when (eq (car-safe option) :include) collect (cadr option))
             (mapcar #'romacs-live-forms--slot-name (cl-remove-if #'keywordp slots))
             (cl-loop for option in options
                      when (or (eq option :named) (memq (car-safe option) '(:type :named :initial-offset)))
                      collect option))))
    ('defclass
     (list (nth 2 form)
           (mapcar (lambda (slot) (cons (romacs-live-forms--slot-name slot)
                                        (and (consp slot) (plist-get (cdr slot) :allocation))))
                   (nth 3 form))
           nil))))

(defun romacs-live-forms-literal-p (form)
  "Whether FORM evaluates to itself or is quoted: an initial value a live
update may compare with a variable's value without running anything."
  (or (memq (car-safe form) '(quote))
      (and (atom form) (or (not (symbolp form)) (memq form '(nil t)) (keywordp form)))))

(defun romacs-live-forms-literal-value (form)
  "The value of FORM, a literal (`romacs-live-forms-literal-p')."
  (if (eq (car-safe form) 'quote) (cadr form) form))

(defun romacs-live-forms-initial (form)
  "The initial-value form FORM (a `defvar', `defconst' or `defcustom')
gives, as (VALUE-FORM), or nil when it gives none."
  (and (memq (car-safe form) '(defvar defconst defvar-local defcustom))
       (cddr form) (list (nth 2 form))))

(defun romacs-live-forms-face-spec (form)
  "The spec form a `defface' FORM gives."
  (and (eq (car-safe form) 'defface) (nth 2 form)))

(defun romacs-live-forms-foreign-setqs (forms defined)
  "The variables FORMS set at top level with `setq' or `setq-default'
that are not in DEFINED, the variables the same forms define."
  (delete-dups
   (cl-loop for form in (romacs-live-forms-flatten forms)
            when (memq (car-safe form) '(setq setq-default))
            append (cl-loop for (var _) on (cdr form) by #'cddr
                            unless (memq var defined) collect var))))

(defun romacs-live-forms--binding (value)
  "VALUE, a key's binding, as data a worker VM can send: a command symbol,
a string or vector (a keyboard macro), or `:other' for a binding no symbol
names (a lambda, a menu item with one)."
  (cond ((or (symbolp value) (stringp value) (vectorp value)) value)
        ((and (eq (car-safe value) 'menu-item) (symbolp (nth 2 value))) (nth 2 value))
        (t :other)))

(defun romacs-live-forms-beside-itself-p (forms)
  "Whether FORMS find a file beside their own: a call of
`file-name-directory' on an expression naming `load-file-name' or
`load-true-file-name'.  Quoted data (a form written out for another file
to run) is no call."
  (cl-labels ((names-p (x) (cond ((memq x '(load-file-name load-true-file-name)) t)
                                 ((memq (car-safe x) '(quote function \`)) nil)
                                 ((consp x) (or (names-p (car x)) (names-p (cdr x))))))
              (unquoted (x) (cond ((not (consp x)) nil)
                                  ;; A template's unquotes run: walk those.
                                  ((memq (car x) '(\, \,@)) (walk (cadr x)))
                                  (t (cl-loop for tail on x thereis (and (consp tail) (unquoted (car tail)))))))
              (walk (x) (cond ((not (consp x)) nil)
                              ((eq (car x) 'quote) nil)
                              ((eq (car x) '\`) (unquoted (cadr x)))
                              ((and (eq (car x) 'file-name-directory) (names-p (cdr x))) t)
                              (t (cl-loop for tail on x
                                          thereis (if (consp tail) (walk (car tail)) nil))))))
    (cl-some #'walk forms)))

(defun romacs-live-forms-keymap-flat (map &optional raw)
  "MAP's own bindings as an alist of key vectors to bindings, each as
`romacs-live-forms--binding' makes it, or with RAW the binding itself.
Prefix keymaps are followed and their keys listed whole; the parent is
not."
  (let ((flat nil) (seen nil))
    (cl-labels ((walk (map prefix)
                  (unless (memq map seen)
                    (push map seen)
                    (map-keymap-internal
                     (lambda (event value)
                       (let ((key (vconcat prefix (vector event))))
                         (if (and (keymapp value) (not (symbolp value)))
                             (walk value key)
                           (push (cons key (if raw value (romacs-live-forms--binding value))) flat))))
                     map))))
      (walk map []))
    (nreverse flat)))

(defun romacs-live-forms-user-bindings (current defaults)
  "The bindings of CURRENT, a keymap flattened RAW, that DEFAULTS (the same
map as its library alone makes it, flattened) does not have: the keys the
user bound or rebound, each (KEY . BINDING).  A key both bind to what no
symbol names is taken for the library's."
  (cl-loop for (key . binding) in current
           for default = (assoc key defaults)
           when (or (null default)
                    (not (equal (cdr default) (romacs-live-forms--binding binding))))
           collect (cons key binding)))

(provide 'romacs-live-forms)
;;; romacs-live-forms.el ends here
