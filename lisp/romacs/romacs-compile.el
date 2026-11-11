;;; romacs-compile.el --- Compile a Lisp file for compiled loads  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The front end of Romacs's compiled Lisp, its analogue of
;; `byte-compile-file', without bytecode: its artifacts hold VM2 code.
;; A compile process (`compiled_lisp' in Rust) calls
;; `romacs-compile--begin' on a file, then `romacs-compile--next' until
;; it returns `:eof'.  Each call reads one top-level form and walks it by
;; bytecomp's file-compilation rules (`byte-compile-toplevel-file-form'):
;;
;; - split: expand the head only, and treat each child of a resulting
;;   `progn' as a top-level form (`byte-compile-recurse-toplevel');
;; - preprocess each piece: `macroexpand-all' in the walk's macro
;;   environment, compiler macros included, then GNU's compiled
;;   docstrings and defsubst inlining, calls of pure functions on
;;   constants folded as GNU's optimizer folds them, and a literal
;;   lambda given to `mapcar' or `mapc' made the body of a loop;
;; - dispatch on the preprocessed form's head, as bytecomp's
;;   `byte-hunk-handler's do, with their compile-time effects.
;;
;; It returns the forms to compile, each into one unit, with what the
;; loader records of each.  Romacs's compiler then compiles them to VM2
;; code; no byte code is made.  Compile-time evaluation happens here,
;; once per compile, never at a compiled load.

;;; Code:

(require 'bytecomp)
;; The `pure' property GNU's optimizer folds calls by.
(require 'byte-opt)

(defvar romacs-compile--input nil "The buffer of the file being compiled.")
(defvar romacs-compile--file nil "The file being compiled.")
(defvar romacs-compile--ordinal 0 "How many top-level forms were read.")
(defvar romacs-compile--line '(1 . 1)
  "The line of a buffer position, as (POSITION . LINE), kept as reading advances.")
(defvar romacs-compile--emitted nil "The forms to compile from the current top-level form.")
(defvar romacs-compile--functions nil
  "Inline-eligible functions this file defined: (NAME . FUNCTION).
Bytecomp's `byte-compile-function-environment' for them: the latest
compile-time definition, held here and never installed.")
(defvar romacs-compile--not-inlined nil
  "Inline-eligible functions called without a definition to inline.")

(defun romacs-compile--lexenv ()
  "The lexical environment compile-time `eval' uses, as bytecomp's."
  (when lexical-binding
    (or (append macroexp--dynvars byte-compile-bound-variables) t)))

(defun romacs-compile--eval-when-compile (&rest body)
  "The walk's `eval-when-compile': evaluate BODY now, give its value quoted."
  (let ((result nil))
    (byte-compile-recurse-toplevel
     (macroexp-progn body)
     (lambda (form)
       (setq result (eval (romacs-compile--preprocess form) (romacs-compile--lexenv)))))
    (list 'quote result)))

(defun romacs-compile--eval-and-compile (&rest body)
  "The walk's `eval-and-compile': evaluate BODY now and keep it."
  (byte-compile-recurse-toplevel
   (macroexp-progn body)
   (lambda (form)
     (let ((expanded (macroexpand--all-toplevel form macroexpand-all-environment)))
       (eval expanded (romacs-compile--lexenv))
       expanded))))

(defun romacs-compile--begin (file)
  "Visit FILE for compiling, as `byte-compile-file' does (bytecomp.el:2240).
Return nil, or why the file is not compiled."
  (let ((buffer (get-buffer-create " *Compiler Input*")))
    (with-current-buffer buffer
      (erase-buffer)
      (setq buffer-file-coding-system nil)
      (set-buffer-multibyte t)
      (insert-file-contents file)
      (when (or (eq last-coding-system-used 'no-conversion)
                (eq (coding-system-type last-coding-system-used) 5))
        (set-buffer-multibyte nil))
      (let ((buffer-file-name file)
            (dmm (default-value 'major-mode))
            (enable-local-variables :safe)
            (enable-local-eval nil))
        (unwind-protect
            (progn
              (setq-default major-mode 'emacs-lisp-mode)
              (delay-mode-hooks (normal-mode t)))
          (setq-default major-mode dmm)))
      (setq buffer-read-only nil)
      (unless (or (local-variable-p 'lexical-binding) no-byte-compile)
        (setq-local lexical-binding nil))
      (setq default-directory (file-name-directory file))
      (goto-char (point-min)))
    ;; `byte-compile-close-variables': this process compiles one file.
    (setq romacs-compile--input buffer
          romacs-compile--file file
          romacs-compile--ordinal 0
          romacs-compile--line (cons (with-current-buffer buffer (point-min)) 1)
          romacs-compile--functions nil
          romacs-compile--not-inlined nil
          byte-compile-current-file file
          ;; What `byte-compile-file' binds: cc-mode's `c-get-current-file'
          ;; reads it to tell compiling from loading.  Nothing is written
          ;; there, and `byte-compile--\#$' stays nil: docstrings stay
          ;; inline in the artifact, never lazy `(#$ . POS)' references.
          byte-compile-dest-file (byte-compile-dest-file file)
          byte-compile-current-buffer buffer
          ;; bytecomp's `declare-function' entry is also what
          ;; `macroexp-compiling-p' looks for: macros (cl-eval-when,
          ;; cl-load-time-value, cl-defstruct) expand differently for
          ;; the compiler.
          byte-compile-macro-environment
          (list (cons 'declare-function #'byte-compile-macroexpand-declare-function)
                (cons 'eval-when-compile #'romacs-compile--eval-when-compile)
                (cons 'eval-and-compile #'romacs-compile--eval-and-compile))
          byte-compile--outbuffer
          (with-current-buffer (get-buffer-create " *Compiler Output*")
            (erase-buffer)
            (current-buffer))
          byte-compile-function-environment nil
          byte-compile-bound-variables nil
          byte-compile-const-variables nil
          overriding-plist-environment nil)
    (when-let* ((reason (buffer-local-value 'no-byte-compile buffer)))
      (format "no-byte-compile: %S" reason))))

(defun romacs-compile--next ()
  "Read and walk the next top-level form.
Return `:eof', or a list of (FORM BYTE END LINE ORDINAL SPECIALS): each
form to compile into a unit, the byte offsets of the top-level form it
came from, its line and ordinal, and the names the file has declared
special so far."
  (with-current-buffer romacs-compile--input
    (let ((case-fold-search nil)
          (float-output-format nil)
          (print-length nil)
          (print-level nil))
      (while (progn (skip-chars-forward " \t\n\f")
                    (= (following-char) ?\;))
        (forward-line 1))
      (if (eobp)
          :eof
        (let* ((begin (point))
               (form (let ((lread--unescaped-character-literals nil))
                       (read (current-buffer))))
               (end (point))
               (ordinal (setq romacs-compile--ordinal (1+ romacs-compile--ordinal)))
               (bol (save-excursion (goto-char begin) (line-beginning-position)))
               (line (+ (cdr romacs-compile--line) (count-lines (car romacs-compile--line) bol)))
               (bytes (1- (position-bytes begin)))
               (end-bytes (1- (position-bytes end))))
          (setq romacs-compile--line (cons bol line))
          (setq romacs-compile--emitted nil)
          (byte-compile-recurse-toplevel
           form
           (lambda (piece)
             (romacs-compile--file-form (romacs-compile--preprocess piece))))
          (romacs-compile--printed)
          (mapcar (lambda (emitted)
                    (list (car emitted) bytes end-bytes line ordinal (cdr emitted)))
                  (nreverse romacs-compile--emitted)))))))

(defun romacs-compile--printed ()
  "Emit the forms a macro printed into `byte-compile--outbuffer'.
`cl-load-time-value' prints its `setq' there, which GNU's `.elc' then
holds before the form being compiled: they come first here too."
  (with-current-buffer byte-compile--outbuffer
    (unless (= (point-min) (point-max))
      (goto-char (point-min))
      (let ((printed nil))
        (while (progn (skip-chars-forward " \t\n\f") (not (eobp)))
          (push (read (current-buffer)) printed))
        (erase-buffer)
        (let ((byte-compile-bound-variables byte-compile-bound-variables)
              (romacs-compile--emitted nil))
          (dolist (form (nreverse printed))
            (romacs-compile--emit (romacs-compile--preprocess form)))
          (setq printed romacs-compile--emitted))
        (setq romacs-compile--emitted (append romacs-compile--emitted printed))))))

(defun romacs-compile--emit (form)
  "Compile FORM into a unit, with the specials declared so far."
  (push (cons form byte-compile-bound-variables) romacs-compile--emitted))

(defvar romacs-compile--forms nil
  "The forms being preprocessed, expanded, innermost first: a form an
`eval-when-compile' preprocesses lies inside the one that holds it.")

(defun romacs-compile--preprocess (form)
  "`byte-compile-preprocess' for the walk: FORM fully expanded."
  (let ((romacs-compile--forms (cons (macroexpand-all form byte-compile-macro-environment)
                                     romacs-compile--forms)))
    (romacs-compile--inline (car romacs-compile--forms))))

(defun romacs-compile--declare (name)
  "`byte-compile--declare-var': NAME is special for the rest of the file."
  (when (symbolp name)
    (push name byte-compile-bound-variables)))

(defun romacs-compile--file-form (form)
  "`byte-compile-file-form': dispatch FORM on GNU's byte-hunk-handlers."
  (pcase form
    (`(,(or 'progn 'prog1) . ,body)
     (mapc #'romacs-compile--file-form body))
    (`(with-no-warnings . ,body)
     (let ((byte-compile-warnings nil))
       (mapc #'romacs-compile--file-form body)))
    (`(internal--with-suppressed-warnings ,warnings . ,body)
     (let ((byte-compile--suppressed-warnings
            (append (cadr warnings) byte-compile--suppressed-warnings)))
       (mapc #'romacs-compile--file-form body)))
    (`(require . ,args)
     (apply #'require (mapcar #'eval args))
     (romacs-compile--emit form))
    (`(autoload . ,args)
     ;; bytecomp.el:2632: an autoload of a macro, all constants, is made now.
     (when (and (seq-every-p #'macroexp-const-p args)
                (memq (eval (nth 4 args)) '(t macro)))
       (eval form))
     (romacs-compile--emit form))
    (`(defvar ,name)
     (romacs-compile--declare name))
    (`(,(or 'defvar 'defconst) ,name . ,_)
     (romacs-compile--declare name)
     (when (eq (car form) 'defconst)
       (push name byte-compile-const-variables))
     (romacs-compile--emit form))
    (`(,(or 'defvaralias 'custom-declare-variable 'define-abbrev-table) ,name . ,_)
     (pcase name (`',symbol (romacs-compile--declare symbol)))
     (romacs-compile--emit form))
    (`(make-obsolete . ,args)
     (romacs-compile--emit form)
     (apply #'make-obsolete (mapcar #'eval args)))
    (`(,(or 'function-put 'define-symbol-prop) ,fun ,prop ,value)
     (when (and (macroexp-const-p fun) (macroexp-const-p prop)
                (or (macroexp-const-p value)
                    (eq (car-safe value) 'function)))
       ;; bytecomp records it in `overriding-plist-environment'; the
       ;; compile process is this file's alone, so it applies it.
       (funcall (car form) (eval fun t) (eval prop t) (eval value (romacs-compile--lexenv))))
     (romacs-compile--emit form))
    (`(defalias ',name ,definition . ,_)
     (romacs-compile--defalias name definition)
     (romacs-compile--emit form))
    (_ (romacs-compile--emit form))))

(defun romacs-compile--defalias (name definition)
  "The compile-time effect of defining NAME as DEFINITION.
A macro becomes callable for the rest of the file
(`byte-compile-macro-environment'); an inline-eligible function's
definition becomes the one later callers inline.  Any other definition
is recorded as t, as `byte-compile-file-form-defalias' records it
(bytecomp.el:5162): later calls are not inlined."
  (pcase definition
    ((or `(cons 'macro ,fun) `'(macro . ,(app macroexp-quote fun)))
     (push (cons name (eval fun (romacs-compile--lexenv))) byte-compile-macro-environment))
    ((or `(function (lambda . ,_)) `(quote (lambda . ,_)))
     (when (romacs-compile--eligible-p name)
       (push (cons name (eval definition (romacs-compile--lexenv))) romacs-compile--functions)))
    (_ (push (cons name t) romacs-compile--functions))))

(defun romacs-compile--eligible-p (name)
  "Whether calls to NAME are inlined: `defsubst' marked it."
  (and (symbolp name)
       (eq (get name 'byte-optimizer) 'byte-compile-inline-expand)))

(defun romacs-compile--definition (name)
  "What a call to the inline function NAME inlines, or nil.
`byte-compile-inline-expand' (byte-opt.el:144): this file's latest
definition, else the function cell's, an alias followed and an autoload
loaded."
  (let ((fn (let ((local (assq name romacs-compile--functions)))
              (if local (cdr local) (symbol-function name)))))
    (when (autoloadp fn)
      (autoload-do-load fn name)
      (setq fn (symbol-function name)))
    (cond ((and (symbolp fn) fn (not (eq fn t)))
           (and (romacs-compile--eligible-p fn) (romacs-compile--definition fn)))
          ((functionp fn) fn))))

(defun romacs-compile--inline (form)
  "FORM, expanded, with calls to inline functions made to a frozen copy.
Each call (F ARGS...) becomes (funcall \\='DEF ARGS...), DEF being F's
current definition as `romacs-compile--definition' finds it, and every
lambda gets GNU's compiled docstring (bytecomp.el:3115)."
  (cond
   ((not (consp form)) form)
   ((memq (car form) '(quote interactive)) form)
   ((eq (car form) 'function)
    (pcase form
      (`(function (lambda ,args . ,body))
       `(function (lambda ,args ,@(romacs-compile--docstring
                                   args (mapcar #'romacs-compile--inline body)))))
      (_ form)))
   ((memq (car form) '(let let*))
    `(,(car form)
      ,(mapcar (lambda (binding)
                 (if (consp binding)
                     (cons (car binding) (mapcar #'romacs-compile--inline (cdr binding)))
                   binding))
               (cadr form))
      ,@(mapcar #'romacs-compile--inline (cddr form))))
   ((eq (car form) 'cond)
    (cons 'cond (mapcar (lambda (clause) (mapcar #'romacs-compile--inline clause))
                        (cdr form))))
   ((eq (car form) 'condition-case)
    `(condition-case ,(nth 1 form) ,(romacs-compile--inline (nth 2 form))
       ,@(mapcar (lambda (handler)
                   (cons (car handler) (mapcar #'romacs-compile--inline (cdr handler))))
                 (nthcdr 3 form))))
   ((not (proper-list-p form)) form)
   ((romacs-compile--mapping-p form) (romacs-compile--mapping form))
   (t
    (let ((head (car form))
          (args (mapcar #'romacs-compile--inline (cdr form))))
      (if (and (symbolp head) (not (special-form-p head)) (not (macrop head))
               (romacs-compile--eligible-p head))
          (let ((fn (romacs-compile--definition head)))
            (if fn
                `(funcall ',fn ,@args)
              (unless (or (memq head romacs-compile--not-inlined)
                          (eq (cdr (assq head romacs-compile--functions)) t))
                (push head romacs-compile--not-inlined))
              (cons head args)))
        (romacs-compile--fold
         (cons (if (consp head) (romacs-compile--inline head) head) args)))))))

(defun romacs-compile--fold (form)
  "FORM, a call whose arguments are walked, evaluated now if it may be.
`byte-optimize-constant-args' (byte-opt.el): a call of a function GNU
marks `pure' whose arguments are all constants becomes its value,
quoted.  A call that signals is left to signal when it runs, and so is
one whose value the compiled store cannot hold
\(`romacs-compile--storable-p')."
  (if (and (symbolp (car form))
           (function-get (car form) 'pure)
           (seq-every-p #'macroexp-const-p (cdr form)))
      (condition-case nil
          (let ((value (eval form t)))
            (if (romacs-compile--storable-p value) (list 'quote value) form))
        (error form))
    form))

(defconst romacs-compile--budget 400
  "The most conses or elements a walk of a form or value looks at.
It bounds a folded constant, and a lambda's body made a loop's body:
that body is compiled twice, as the loop's and as the lambda's a
redefined or advised `mapcar' is called with.")

(defun romacs-compile--storable-p (value)
  "Whether VALUE is a constant the compiled store holds as itself.
Numbers, interned symbols, strings without text properties and
bool-vectors, and lists and vectors of those, at most `romacs-compile--budget' of them: no hash
table, marker, buffer or other object of the session, and no cycle."
  (let ((budget romacs-compile--budget) (pending (list value)) (ok t))
    (while (and ok pending)
      (let ((v (pop pending)))
        (setq budget (1- budget))
        (cond ((< budget 0) (setq ok nil))
              ((and (stringp v) (null (text-properties-at 0 v)) (null (next-property-change 0 v))))
              ((or (numberp v) (bool-vector-p v)
                   (and (symbolp v) (eq v (intern-soft (symbol-name v))))))
              ((consp v) (push (car v) pending) (push (cdr v) pending))
              ((vectorp v) (setq pending (append v pending)))
              (t (setq ok nil)))))
    ok))

(defun romacs-compile--size (form)
  "How many conses FORM has, counted up to `romacs-compile--budget' and one more.
A larger or circular form stops the count there."
  (let ((count 0) (pending (list form)))
    (while (and pending (<= count romacs-compile--budget))
      (let ((next (pop pending)))
        (while (and (consp next) (<= count romacs-compile--budget))
          (setq count (1+ count))
          (push (car next) pending)
          (setq next (cdr next)))))
    count))

(defun romacs-compile--declares-p (form symbol)
  "Whether FORM is a declaration that makes SYMBOL special.
`defvar' and `defconst' of it, and `defvaralias' and
`custom-declare-variable' of it quoted."
  (pcase form
    (`(,(or 'defvar 'defconst) ,(pred (eq symbol)) . ,_) t)
    (`(,(or 'defvaralias 'custom-declare-variable) (quote ,(pred (eq symbol))) . ,_) t)))

(defun romacs-compile--special-p (symbol)
  "Whether a `let' of SYMBOL here could bind it dynamically.
Special globally, declared special earlier in the file or by an
enclosing form being expanded (`macroexp--dynvars'), or declared
anywhere in the forms being preprocessed (`romacs-compile--declares-p')."
  (or (special-variable-p symbol)
      (memq symbol byte-compile-bound-variables)
      (and (boundp 'macroexp--dynvars) (memq symbol macroexp--dynvars))
      (let ((pending (copy-sequence romacs-compile--forms)) (found nil) (budget 100000))
        (while (and pending (not found) (> budget 0))
          (let ((next (pop pending)))
            (while (and (consp next) (not found) (> budget 0))
              (setq budget (1- budget))
              (if (romacs-compile--declares-p next symbol)
                  (setq found t))
              (push (car next) pending)
              (setq next (cdr next)))))
        ;; Forms too large to search are taken to declare it.
        (or found (<= budget 0)))))

(defun romacs-compile--mapping-p (form)
  "Whether FORM is (mapcar|mapc (function (lambda (ARG) BODY...)) SEQ).
Only in a `lexical-binding' file, for a lambda of one required argument
that no declaration makes special (a `let' of it would bind it
dynamically, the parameter not), with a body that is not documentation,
`interactive' or `declare'."
  (pcase form
    (`(,(or 'mapcar 'mapc) (function (lambda (,(and arg (pred symbolp))) . ,body)) ,_)
     (and lexical-binding
          body
          (not (memq arg '(&optional &rest)))
          (not (and (stringp (car body)) (cdr body)))
          (not (memq (car-safe (car body)) '(interactive declare :documentation)))
          (not (romacs-compile--special-p arg))))))

(defun romacs-compile--mapping (form)
  "FORM, a `romacs-compile--mapping-p' call, as a loop over a list.
The loop runs while `mapcar' or `mapc' is still its own definition (the
`romacs--definition' property the runtime puts on it) and SEQ is a
proper list, and reads as many elements as SEQ had when it began, as the
function does.  Anything else calls the function with the lambda: an
advised or redefined one, a vector or string, and a dotted or circular
list, for which it signals.  The body is walked once; where the walked
body is larger than `romacs-compile--budget', the call is kept, so that
nested mappings cannot multiply."
  (pcase-let* ((`(,fn (function (lambda (,arg) . ,source)) ,seq) form)
               (body (mapcar #'romacs-compile--inline source))
               (seq (romacs-compile--inline seq))
               (lambda `(function (lambda (,arg) ,@(romacs-compile--docstring (list arg) body)))))
    (if (> (romacs-compile--size body) romacs-compile--budget)
        `(,fn ,lambda ,seq)
      (let ((list (make-symbol "list"))
            (tail (make-symbol "tail"))
            (count (make-symbol "count"))
            (results (make-symbol "results")))
        `(let* ((,list ,seq)
                (,count (and (eq (symbol-function ',fn) (get ',fn 'romacs--definition))
                             (proper-list-p ,list))))
           (if ,count
               (let ((,tail ,list) ,@(if (eq fn 'mapcar) `((,results nil))))
                 (while (and (> ,count 0) (consp ,tail))
                   ,(let ((step `(let ((,arg (car ,tail))) ,@body)))
                      (if (eq fn 'mapcar) `(setq ,results (cons ,step ,results)) step))
                   (setq ,tail (cdr ,tail) ,count (1- ,count)))
                 ,(if (eq fn 'mapcar) `(nreverse ,results) list))
             (,fn ,lambda ,list)))))))

(defun romacs-compile--docstring (args body)
  "BODY of a lambda with ARGS, its docstring as bytecomp compiles it.
In a `lexical-binding' file a lambda with arguments gets
`help-add-fundoc-usage', so its `(fn ARGS)' survives compilation."
  (if (not (and lexical-binding args))
      body
    (cond
     ((and (cdr body) (stringp (car body)))
      (cons (help-add-fundoc-usage (car body) args) (cdr body)))
     ((eq (car-safe (car body)) :documentation) body)
     (t (cons (help-add-fundoc-usage nil args) (or body '(nil)))))))

(defun romacs-compile--end ()
  "The compile's report: the inline-eligible functions it could not inline."
  (prog1 (list :not-inlined romacs-compile--not-inlined)
    (dolist (buffer (list romacs-compile--input byte-compile--outbuffer))
      (when (buffer-live-p buffer)
        (kill-buffer buffer)))))

(provide 'romacs-compile)
;;; romacs-compile.el ends here
