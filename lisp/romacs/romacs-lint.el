;;; romacs-lint.el --- Check Lisp against the machine that runs it  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A lint that asks the running machine instead of guessing.  It loads
;; each file it is given, expands every form with the macros now defined,
;; and holds what the code names to what the machine has:
;;
;;   error    a file that does not read or load; a call to a function
;;            nothing defines; a call with a number of arguments the
;;            function does not take; a `declare-function' that is not
;;            true (its file does not define the function, or its
;;            argument list is not the function's).
;;   warning  a free variable; an unused lexical variable; an obsolete
;;            function; a missing or nil `lexical-binding' cookie; a
;;            docstring line over 80 columns; a string that pins a line
;;            of a file of this repository.
;;   hint     what the machine offers instead: a `romacs-lint-hint' the
;;            owner of a better path put on the function called, work
;;            that reaches no state and so can run in a worker, a
;;            `cl-defstruct' whose layout left its base's.
;;
;; A hint's text comes from the machine: a primitive's effect class
;; (`romacs--subr-effect'), a symbol's properties, a docstring.  The
;; owner of a faster path declares it beside its definition:
;;
;;   (put 'syntax-ppss 'romacs-lint-hint "...")
;;
;; and every caller of `syntax-ppss' is told.  No list is kept here.
;;
;; One finding is one line, with its fix:
;;
;;   FILE:LINE: LEVEL: in DEFINITION: WHAT; fix: HOW
;;
;; `tools/romacs-lint' runs `romacs-lint-batch' in a batch machine.

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(defconst romacs-lint--function-binders
  '(defun defmacro defsubst defalias cl-defun cl-defmacro cl-defgeneric
     cl-defmethod define-minor-mode define-derived-mode define-inline)
  "Heads whose second element names the function a top-level form defines.")

(defconst romacs-lint--binders
  (append romacs-lint--function-binders '(defvar defconst defcustom defface cl-defstruct ert-deftest))
  "Heads whose second element names what a top-level form defines.")

;;;; Reading

(defun romacs-lint--read (text)
  "TEXT's top-level forms as (FORMS . ERROR): each form (LINE . FORM),
ERROR what stopped the reader short of the end, or nil."
  (with-temp-buffer
    (insert text)
    (goto-char (point-min))
    (let (forms problem)
      (condition-case err
          (while (progn (while (progn (skip-chars-forward " \t\n\f\r") (eq (char-after) ?\;)) (forward-line))
                        (not (eobp)))
            (let ((line (line-number-at-pos)))
              (setq problem (format "the form at line %d: " line))
              (push (cons line (read (current-buffer))) forms)
              (setq problem nil)))
        (end-of-file (setq problem (concat problem "the file ends inside it")))
        (error (setq problem (concat problem (error-message-string err)))))
      (cons (nreverse forms) problem))))

(defun romacs-lint--file-text (file)
  "FILE's text."
  (with-temp-buffer (insert-file-contents file) (buffer-string)))

(defun romacs-lint--cookie (text)
  "The `lexical-binding' TEXT's first line sets, as a string, or nil."
  (let ((line (car (split-string text "\n"))))
    (and (string-match "-\\*-.*lexical-binding:[ \t]*\\([^; \t]+\\)" line) (match-string 1 line))))

(defun romacs-lint--name (form &optional binders)
  "What top-level FORM defines, or nil.  BINDERS are the heads that count,
default `romacs-lint--binders'."
  (and (consp form) (memq (car form) (or binders romacs-lint--binders)) (consp (cdr form))
       (let ((name (cadr form)))
         (cond ((symbolp name) name)
               ((and (consp name) (eq (car name) 'quote)) (cadr name))
               ((consp name) (car name))))))

;;;; Walking expanded code

(defun romacs-lint--guards (condition &optional negated)
  "What CONDITION being true establishes: (function . F) for (fboundp \\='F),
(variable . V) for (boundp \\='V).  With NEGATED, what its being false does."
  (pcase condition
    (`(,(or 'not 'null) ,inner) (romacs-lint--guards inner (not negated)))
    (`(,(or 'fboundp 'functionp) (,(or 'quote 'function) ,(and name (pred symbolp))))
     (unless negated (list (cons 'function name))))
    (`(boundp (quote ,(and name (pred symbolp))))
     (unless negated (list (cons 'variable name))))
    ;; All of an `and' hold when it is true, none of an `or' when it is false.
    (`(,(and head (or 'and 'or)) . ,conditions)
     (and (eq (eq head 'or) (and negated t)) (proper-list-p conditions)
          (mapcan (lambda (each) (romacs-lint--guards each negated)) conditions)))))

(defun romacs-lint--walk (form env known special emit)
  "Report what macroexpanded FORM names to EMIT.
ENV is the lexical environment, each entry (NAME . CELL) with CELL's car
non-nil once NAME is read.  KNOWN is what the guards around FORM
establish (`romacs-lint--guards').  SPECIAL holds the variables the file
declares.  EMIT takes (call F ARGC KNOWN), (function F KNOWN), (free V),
(unused V), (state) where FORM reads, writes or binds a variable that is
not lexical or saves the buffer's state, and (enter) and (leave) around
the body of each function written out."
  (cl-labels ((walk (form &optional env* known*) (romacs-lint--walk form (or env* env) (or known* known) special emit))
              (specialp (name) (or (special-variable-p name) (gethash name special)))
              ;; ENV with ARGLIST's parameters bound, each as used.  A special
              ;; one is bound dynamically: state, and no entry.
              (parameters (arglist)
                (let ((env env))
                  (dolist (name (and (proper-list-p arglist) arglist) env)
                    (cond ((memq name '(&optional &rest)))
                          ((specialp name) (funcall emit '(state)))
                          (t (push (cons name (list t)) env))))))
              (variable (name)
                (let ((cell (assq name env)))
                  (cond (cell (setcar (cdr cell) t))
                        (t (funcall emit '(state))
                           (unless (or (boundp name) (specialp name) (member (cons 'variable name) known))
                             (funcall emit (list 'free name))))))))
    (pcase form
      ((pred keywordp) nil)
      ((or 'nil 't) nil)
      ((pred symbolp) (variable form))
      ((pred atom) nil)
      ((guard (not (proper-list-p form))) nil)
      (`(quote . ,_) nil)
      (`(romacs-lint--quiet . ,_) nil)
      ;; A string is a specification; a list is code that reads the arguments.
      (`(interactive . ,arguments) (when (consp (car arguments)) (walk (car arguments))))
      (`(function (lambda ,arglist . ,body))
       (funcall emit '(enter))
       (let ((env (parameters arglist)))
         (dolist (form body) (walk form env)))
       (funcall emit '(leave)))
      (`(function ,(and name (pred symbolp))) (funcall emit (list 'function name known)))
      (`(function . ,_) nil)
      (`(,(and head (or 'let 'let*)) ,bindings . ,body)
       (let ((inner env) cells)
         (dolist (binding (and (proper-list-p bindings) bindings))
           (let ((name (if (consp binding) (car binding) binding)))
             (when (consp binding) (walk (cadr binding) (if (eq head 'let*) inner env)))
             (if (specialp name)
                 (funcall emit '(state))
               (let ((cell (cons name (list nil))))
                 (push cell cells)
                 (push cell inner)))))
         (dolist (form body) (walk form inner))
         (dolist (cell cells)
           (unless (or (cadr cell) (string-prefix-p "_" (symbol-name (car cell))))
             (funcall emit (list 'unused (car cell)))))))
      ;; An assignment is a use: only a name nothing binds is free.
      (`(setq . ,pairs)
       (cl-loop for (name value) on pairs by #'cddr
                do (walk value) (variable name)))
      (`(cond . ,clauses)
       (let ((known known))
         (dolist (clause clauses)
           (when (proper-list-p clause)
             (walk (car clause) nil known)
             (let ((known (append (romacs-lint--guards (car clause)) known)))
               (dolist (form (cdr clause)) (walk form nil known)))
             (setq known (append (romacs-lint--guards (car clause) t) known))))))
      (`(if ,condition ,then . ,else)
       (walk condition)
       (walk then nil (append (romacs-lint--guards condition) known))
       (let ((known (append (romacs-lint--guards condition t) known)))
         (dolist (form else) (walk form nil known))))
      (`(while ,condition . ,body)
       (walk condition)
       (let ((known (append (romacs-lint--guards condition) known)))
         (dolist (form body) (walk form nil known))))
      ;; Each form of an `and' runs with those before it true, of an `or' false.
      (`(,(and head (or 'and 'or)) . ,forms)
       (let ((known known))
         (dolist (form forms)
           (walk form nil known)
           (setq known (append (romacs-lint--guards form (eq head 'or)) known)))))
      (`(condition-case ,var ,body . ,handlers)
       (walk body)
       (let ((env (if var (cons (cons var (list t)) env) env)))
         (dolist (handler handlers)
           (dolist (form (cdr-safe handler)) (walk form env)))))
      (`(,(or 'defvar 'defconst) ,_ . ,rest) (when rest (walk (car rest))))
      (`((lambda ,arglist . ,body) . ,arguments)
       (dolist (argument arguments) (walk argument))
       (let ((env (parameters arglist)))
         (dolist (form body) (walk form env))))
      (`(,(and head (pred symbolp)) . ,arguments)
       (cond ((not (special-form-p head)) (funcall emit (list 'call head (length arguments) known)))
             ((memq head '(save-excursion save-restriction save-current-buffer)) (funcall emit '(state))))
       (dolist (argument arguments) (walk argument)))
      (_ nil))))

(defun romacs-lint--declared-variables (form into)
  "Put each variable expanded FORM declares with `defvar' in table INTO."
  (when (and (consp form) (proper-list-p form))
    (if (and (memq (car form) '(defvar defconst)) (symbolp (cadr form)))
        (puthash (cadr form) t into)
      (unless (eq (car form) 'quote)
        (dolist (part form) (romacs-lint--declared-variables part into))))))

(defun romacs-lint--expand (form)
  "FORM fully macroexpanded, a `with-no-warnings' body left unjudged."
  (macroexpand-all form '((with-no-warnings . (lambda (&rest body) (cons 'romacs-lint--quiet body)))
                          (with-suppressed-warnings . (lambda (_warnings &rest body) (cons 'romacs-lint--quiet body))))))

;;;; What the machine says of a function

(defun romacs-lint--arity (function)
  "FUNCTION's (MIN . MAX), MAX `many' when unbounded; nil when unknown.
An autoload is not loaded to learn it."
  (let ((definition (indirect-function function)))
    (and definition (not (autoloadp definition)) (not (macrop definition))
         (ignore-errors (func-arity definition)))))

(defun romacs-lint--arglist-arity (arglist)
  "The (MIN . MAX) ARGLIST declares."
  (let ((min 0) (max 0) (state 'required))
    (dolist (name arglist)
      (pcase name
        ('&optional (setq state 'optional))
        ((or '&rest '&key '&body) (setq state 'rest max 'many))
        (_ (pcase state
             ('required (setq min (1+ min) max (1+ max)))
             ('optional (setq max (1+ max)))))))
    (cons min max)))

(defun romacs-lint--takes-p (arity argc)
  "Non-nil when a function of ARITY takes ARGC arguments."
  (and (>= argc (car arity)) (or (not (integerp (cdr arity))) (<= argc (cdr arity)))))

(defun romacs-lint--similar (name)
  "The defined function whose name is nearest unknown NAME's, or nil."
  (let* ((text (symbol-name name)) (limit (max 2 (/ (length text) 5))) best)
    (mapatoms (lambda (symbol)
                (when (and (fboundp symbol) (< (abs (- (length (symbol-name symbol)) (length text))) limit))
                  (let ((distance (string-distance text (symbol-name symbol))))
                    (when (and (<= distance limit) (or (null best) (< distance (cdr best))))
                      (setq best (cons symbol distance)))))))
    (car best)))

(defun romacs-lint--effect (function)
  "The effect class of primitive FUNCTION (`pure', `reads', ...), or nil."
  (let ((definition (indirect-function function)))
    (and (subrp definition) (fboundp 'romacs--subr-effect)
         (funcall 'romacs--subr-effect definition))))

(defun romacs-lint--rust-file (function)
  "The Rust file that defines primitive FUNCTION, or nil."
  (let ((definition (indirect-function function)))
    (and (subrp definition) (fboundp 'romacs--rust-definition)
         (car (funcall 'romacs--rust-definition definition nil)))))

;;;; declare-function

(defun romacs-lint--occurrences (head form)
  "Every list in unexpanded FORM whose car is HEAD, quoted data apart."
  (cond ((not (and (consp form) (proper-list-p form))) nil)
        ((eq (car form) 'quote) nil)
        ((eq (car form) head) (list form))
        (t (mapcan (lambda (part) (romacs-lint--occurrences head part)) form))))

(defun romacs-lint--declarations (form line)
  "Every `declare-function' in unexpanded FORM, as (LINE FN FILE ARGLIST)."
  (mapcar (lambda (declaration)
            (list line (nth 1 declaration) (nth 2 declaration)
                  (if (nthcdr 3 declaration) (nth 3 declaration) t)))
          (romacs-lint--occurrences 'declare-function form)))

(defun romacs-lint--require-all (forms)
  "Load each feature FORMS require anywhere, as running them would:
a function that requires its library as it starts may call into it."
  (dolist (form forms)
    (pcase-dolist (`(,_ ,feature . ,_) (romacs-lint--occurrences 'require (cdr form)))
      (pcase feature
        (`(quote ,(and name (pred symbolp))) (ignore-errors (require name nil t)))))))

(defun romacs-lint--rust-declares-p (file function root)
  "Non-nil when a Rust file of the tree at ROOT whose path ends in FILE
registers primitive FUNCTION."
  (let ((registration (format "\\(?:name = \\|(\\)\"%s\"" (regexp-quote (symbol-name function)))))
    (cl-some (lambda (path)
               (let ((path (expand-file-name path root)))
                 (and (file-readable-p path) (string-match-p registration (romacs-lint--file-text path)))))
             (ignore-errors
               (process-lines "git" "-C" root "ls-files" "--cached" "--others" "--exclude-standard"
                              "--" (concat "*/" file) file)))))

(defun romacs-lint--declaration (function file arglist root)
  "What is untrue in (declare-function FUNCTION FILE ARGLIST), or nil.
ROOT is the source tree a Rust FILE is relative to."
  (let ((rust (romacs-lint--rust-file function)))
    (or (cond
         ((not (stringp file)) nil)
         ((string-suffix-p ".rs" file)
          (cond ((and rust (or (equal rust file) (string-suffix-p (concat "/" file) rust))) nil)
                ((not root) nil)
                ((romacs-lint--rust-declares-p file function root) nil)
                (t (format "declare-function `%s' names %s, which registers no such primitive; fix: %s"
                           function file (if rust (format "name %s" rust) "name the file of its defsubr")))))
         ((string-suffix-p ".c" file)
          (unless (subrp (indirect-function function))
            (format "declare-function `%s' names C file %s, and this machine has no such primitive; fix: guard the call with `fboundp' and drop the declaration, or implement it" function file)))
         (rust (format "declare-function `%s' names \"%s\", but it is a primitive; fix: name %s" function file rust))
         ;; GNU's word for a library outside the tree: nothing to hold it to.
         ((string-prefix-p "ext:" file) nil)
         ;; A function the world was built with has no file on record, and its
         ;; library need not be on the load path: there is nothing to hold it to.
         ((fboundp function)
          (let ((home (symbol-file function 'defun)))
            (when (and home (not (equal (file-name-base home) (file-name-base file))))
              (format "declare-function `%s' names \"%s\", but %s defines it; fix: name that library" function file (file-name-base home)))))
         ((not (locate-library file))
          (format "declare-function `%s' names \"%s\", which is not on the load path; fix: name the library that defines it" function file))
         ((progn (ignore-errors (load file nil t)) (fboundp function)) nil)
         (t (format "declare-function `%s': loading \"%s\" does not define it; fix: name the library that does, or correct the name" function file)))
        (let ((real (and (listp arglist) (romacs-lint--arity function))))
          ;; (0 . many) is what a generic function or an advised one answers.
          (when (and real (not (equal real '(0 . many)))
                     (not (equal (romacs-lint--arglist-arity arglist)
                                      (cons (car real) (if (integerp (cdr real)) (cdr real) 'many)))))
            (format "declare-function `%s' gives arguments %S, but it takes %S; fix: copy the definition's argument list" function arglist real))))))

;;;; Hints the machine holds

(defun romacs-lint--call-hint (function)
  "A hint for a call to FUNCTION, from what the machine holds, or nil."
  (let ((declared (get function 'romacs-lint-hint)))
    (and (stringp declared) (format "`%s': %s" function declared))))

(defun romacs-lint--obsolete (function)
  "What to say of a call to obsolete FUNCTION, or nil."
  (pcase (get function 'byte-obsolete-info)
    (`(,new ,_ ,when)
     (format "`%s' is obsolete%s; fix: %s" function (if when (format " since %s" when) "")
             (cond ((stringp new) new) (new (format "use `%s'" new)) (t "see its docstring"))))))

(defun romacs-lint--stateless-p (name callees &optional visiting)
  "Non-nil when function NAME reaches no machine state.
CALLEES maps each function of the file to the functions it calls: NAME
is stateless when each of those is a stateless function of the file, a
primitive whose effect is `pure' or `mutates-args' (it touches what it
is given and nothing else), or a function byte-opt.el knows as `pure'.
VISITING are the functions being judged."
  (let ((entry (assq name callees)))
    (if (not entry)
        (or (memq (romacs-lint--effect name) '(pure mutates-args))
            ;; byte-opt.el's word for a function no defsubr classifies.
            (function-get name 'pure))
      (cl-every (lambda (callee)
                  (or (eq callee name) (memq callee visiting)
                      (romacs-lint--stateless-p callee callees (cons name visiting))))
                (cdr entry)))))

(defun romacs-lint--worker-hints (definitions callees)
  "Hints for each of DEFINITIONS, (LINE . NAME), that can run in a worker:
a function of some size that reaches no machine state, by CALLEES."
  (and (fboundp 'isolate-spawn) (fboundp 'romacs--subr-effect) (require 'byte-opt nil t)
       (cl-loop for (line . name) in definitions
                when (and (> (length (cdr (assq name callees))) 7)
                          (romacs-lint--stateless-p name callees))
                collect (list line 'hint
                              (format "in %s: reaches no machine state (all it calls depends on its arguments alone), so large inputs can run in a worker, off the main VM; fix: (isolate-spawn #'%s ARGS...), see `isolate-spawn'" name name)))))

;;;; Struct layouts

(defun romacs-lint--layouts (forms)
  "The `cl-defstruct' layouts among FORMS, as (NAME OPTIONS . SLOTS):
what decides where an instance keeps each slot."
  (cl-loop for (_line . form) in forms
           when (eq (car-safe form) 'cl-defstruct)
           collect (let* ((head (cadr form)) (name (if (consp head) (car head) head))
                          (options (cl-remove-if-not
                                    (lambda (option) (memq (or (car-safe option) option) '(:include :type :named :initial-offset)))
                                    (and (consp head) (cdr head))))
                          (slots (cddr form)))
                     (cons name (cons options (mapcar (lambda (slot) (if (consp slot) (car slot) slot))
                                                      (if (stringp (car slots)) (cdr slots) slots)))))))

(defun romacs-lint--layout-changes (base candidate)
  "Hints for each struct of CANDIDATE forms laid out other than in BASE forms."
  (let ((before (romacs-lint--layouts base)))
    (cl-loop for (line . form) in candidate
             for layout = (car (romacs-lint--layouts (list (cons line form))))
             for old = (and layout (assq (car layout) before))
             when (and old (not (equal old layout)))
             collect (list line 'hint
                           (format "cl-defstruct `%s' changes layout (was %S, now %S): instances made before a live load keep the old one and their accessors break; fix: let it reach sessions by install and restart, or migrate live instances at load"
                                   (car layout) (cddr old) (cddr layout))))))

;;;; Docstrings

(defun romacs-lint--docstring (form)
  "Top-level FORM's docstring, or nil: a function's, a variable's, a face's."
  (let ((doc (pcase (car-safe form)
               ((or 'defun 'defmacro 'defsubst 'cl-defun 'cl-defmacro 'define-inline) (nth 3 form))
               ((or 'defvar 'defconst 'defcustom 'defface 'defvar-local) (nth 3 form)))))
    (and (stringp doc) doc)))

(defun romacs-lint--wide-docstring-p (doc)
  "Non-nil when a line of DOC that could be wrapped is over 80 columns.
The (fn ...) line and a line that is one long word are as they must be."
  (cl-some (lambda (line)
             (and (> (string-width line) 80)
                  (not (string-prefix-p "(fn " line))
                  (string-match-p "[^ ] +[^ ]" line)))
           (split-string doc "\n")))

;;;; Text checks

(defun romacs-lint--line-pins (text root)
  "Warnings for each string in TEXT that pins a line of a file under ROOT."
  (and root
       (with-temp-buffer
         (insert text)
         (goto-char (point-min))
         (cl-loop while (re-search-forward "\"\\([A-Za-z0-9_./-]+/[A-Za-z0-9_.-]+\\.\\(?:rs\\|el\\|py\\)\\):[0-9]+" nil t)
                  when (file-exists-p (expand-file-name (match-string 1) root))
                  collect (list (line-number-at-pos) 'warning
                                (format "pins a line of %s, which moves when anything above it changes; fix: name the declaration by its file and its text"
                                        (match-string 1)))))))

(defvar romacs-lint-root nil
  "The source tree files are judged in, when not the one they are in.
A base's text, kept outside the tree, is linted with this set.")

(defun romacs-lint--root (file)
  "The source tree FILE is judged in: `romacs-lint-root', else the
directory above it that holds a Cargo.lock."
  (or romacs-lint-root
      (let ((root (locate-dominating-file file "Cargo.lock")))
        (and root (expand-file-name root)))))

(defun romacs-lint--base-text (file root revision)
  "FILE's text at git REVISION of the tree at ROOT, or nil."
  (and root revision
       (with-temp-buffer
         (and (eql 0 (ignore-errors
                       (call-process "git" nil t nil "-C" root "show"
                                     (format "%s:%s" revision (file-relative-name file root)))))
              (buffer-string)))))

;;;; One file

(defun romacs-lint--judge-calls (events declared own)
  "Findings for the call and function EVENTS of one top-level form.
DECLARED are the functions the file declares, OWN the ones it defines."
  (let (findings (seen (make-hash-table :test #'equal)))
    (pcase-dolist (`(,kind ,name ,argc ,known) events)
      (when (eq kind 'function) (setq known argc argc nil))
      ;; A guard excuses the occurrence it is around, not the next one.
      (unless (or (and (not (fboundp name)) (member (cons 'function name) known))
                  (gethash (cons name argc) seen))
        (puthash (cons name argc) t seen)
        (cond
         ((not (fboundp name))
          (unless (or (memq name declared) (memq name own))
            (let ((near (romacs-lint--similar name)))
              (push (list 'error (format "`%s' is not defined: not by this machine, this file, or what it requires; fix: %s"
                                         name (if near (format "did you mean `%s'? else require its library" near)
                                                "require its library, or correct the name")))
                    findings))))
         (t
          (let ((arity (and argc (romacs-lint--arity name))))
            (when (and arity (not (romacs-lint--takes-p arity argc)))
              (push (list 'error (format "`%s' called with %d argument%s, takes %S; fix: match its argument list%s"
                                         name argc (if (= argc 1) "" "s") arity
                                         (let ((doc (ignore-errors (documentation name t))))
                                           (if (and doc (string-match "(fn[^)]*)" doc)) (concat " " (match-string 0 doc)) ""))))
                    findings)))
          (unless (memq name own)
            (when-let* ((text (romacs-lint--obsolete name))) (push (list 'warning text) findings))
            (when-let* ((text (romacs-lint--call-hint name))) (push (list 'hint text) findings)))))))
    (nreverse findings)))

(defun romacs-lint--inner-callees (events)
  "The functions EVENTS name inside a function body: what a definition
calls when it runs, the calls that install it apart.  A body that reads,
writes or binds a variable not its own, or saves the buffer's state,
names `romacs-lint--state' among them, which is no function."
  (let ((depth 0) callees)
    (pcase-dolist (`(,kind ,name) events)
      (pcase kind
        ('enter (setq depth (1+ depth)))
        ('leave (setq depth (1- depth)))
        ('state (when (> depth 0) (cl-pushnew 'romacs-lint--state callees)))
        ((or 'call 'function) (when (> depth 0) (cl-pushnew name callees)))))
    (nreverse callees)))

(defun romacs-lint--form-findings (line form special declared own)
  "Top-level FORM at LINE, as (FINDINGS . CALLEES): what is wrong in it,
and the functions its bodies name.  SPECIAL, DECLARED and OWN are the file's
declared variables, declared functions and own definitions."
  (let* ((name (romacs-lint--name form))
         (where (if name (format "in %s: " name) ""))
         events findings)
    (condition-case err
        (romacs-lint--walk (romacs-lint--expand form) nil nil special (lambda (event) (push event events)))
      (error (push (list 'error (format "does not expand: %s; fix: load what defines its macros first" (error-message-string err))) findings)))
    (setq events (nreverse events))
    (when-let* ((doc (romacs-lint--docstring form)))
      (when (romacs-lint--wide-docstring-p doc)
        (push (list 'warning "docstring has a line wider than 80 columns; fix: wrap it") findings)))
    (pcase-dolist (`(,kind ,variable) events)
      (pcase kind
        ('free (push (list 'warning (format "`%s' is a free variable; fix: `defvar' it, or bind it" variable)) findings))
        ('unused (push (list 'warning (format "lexical variable `%s' is never used; fix: drop it or name it `_%s'; if a callee reads it, it is dynamic: add (defvar %s)" variable variable variable)) findings))))
    (let ((calls (cl-remove-if-not (lambda (event) (memq (car event) '(call function))) events)))
      (cons (mapcar (lambda (finding) (list line (car finding) (concat where (cadr finding))))
                    (delete-dups (append (nreverse findings) (romacs-lint--judge-calls calls declared own))))
            (romacs-lint--inner-callees events)))))

(defun romacs-lint-file (file &optional base-revision load-error)
  "The findings for FILE, each (LINE LEVEL TEXT), in line order.
A Lisp FILE is taken as loaded: `romacs-lint-files' loads it first, and
gives the LOAD-ERROR of one that did not.  BASE-REVISION is the git
revision its struct layouts are held to."
  (let* ((text (romacs-lint--file-text file))
         (root (romacs-lint--root file))
         (pins (romacs-lint--line-pins text root)))
    (if (not (string-suffix-p ".el" file))
        pins
      (pcase-let* ((`(,forms . ,unread) (romacs-lint--read text))
                   (cookie (romacs-lint--cookie text))
                   (special (make-hash-table :test #'eq))
                   (declarations (mapcan (lambda (form) (romacs-lint--declarations (cdr form) (car form))) forms))
                   (declared (mapcar #'cadr declarations))
                   (own (delq nil (mapcar (lambda (form) (romacs-lint--name (cdr form) romacs-lint--function-binders)) forms)))
                   (base (romacs-lint--base-text file root base-revision)))
        (romacs-lint--require-all forms)
        (dolist (form forms)
          (romacs-lint--declared-variables (ignore-errors (romacs-lint--expand (cdr form))) special))
        (sort
         (append
          pins
          (cond (unread (list (list 1 'error (format "does not read: %s; fix: balance the form" unread))))
                (load-error (list (list 1 'error (format "does not load: %s; fix: load it in a batch machine and follow the error" load-error)))))
          (cond ((null cookie)
                 (list (list 1 'warning "no lexical-binding cookie: this machine loads the file lexically, GNU Emacs dynamically; fix: add -*- lexical-binding: t -*- to the first line")))
                ((equal cookie "nil")
                 (list (list 1 'warning "lexical-binding: nil asks for dynamic binding, which nothing else here uses; fix: set it to t and `defvar' the variables callees read"))))
          (cl-loop for (line function library arglist) in declarations
                   for untrue = (romacs-lint--declaration function library arglist root)
                   when untrue collect (list line 'error untrue))
          (and base (romacs-lint--layout-changes (car (romacs-lint--read base)) forms))
          (and (not (equal cookie "nil"))
               (let ((reports (mapcar (lambda (form)
                                        (cons form (romacs-lint--form-findings (car form) (cdr form) special declared own)))
                                      forms)))
                 (append
                  (mapcan (lambda (report) (copy-sequence (cadr report))) reports)
                  (romacs-lint--worker-hints
                   (cl-loop for ((line . form) . _) in reports
                            when (memq (car-safe form) '(defun cl-defun))
                            collect (cons line (romacs-lint--name form)))
                   (cl-loop for ((_line . form) _findings . callees) in reports
                            when (romacs-lint--name form)
                            collect (cons (romacs-lint--name form) callees)))))))
         (lambda (a b) (< (car a) (car b))))))))

;;;; Several files, and the command line

(defun romacs-lint--load (files)
  "Load Lisp FILES, the ones that fail once more after the rest.
Return (FILE . ERROR) for each that still does not load."
  (let ((try (lambda (file) (condition-case err (progn (load file nil t t) nil)
                              (error (cons file (error-message-string err)))))))
    (delq nil (mapcar (lambda (failure) (funcall try (car failure)))
                      (delq nil (mapcar try files))))))

(defun romacs-lint-files (files &optional base-revision)
  "Lint FILES together: ((FILE . FINDINGS)...), FINDINGS as `romacs-lint-file'.
Every Lisp file is loaded first, so each is judged against the others as
they are now, whatever this machine was built with.  The directories of
FILES' source tree come first on `load-path' meanwhile."
  (let* ((files (mapcar #'expand-file-name files))
         (lisp (cl-remove-if-not (lambda (file) (string-suffix-p ".el" file)) files))
         (root (and files (romacs-lint--root (car files))))
         (load-path (append (and root (romacs-lint--source-directories root)) load-path))
         (unloaded (romacs-lint--load lisp)))
    (mapcar (lambda (file)
              (cons file (romacs-lint-file file base-revision (cdr (assoc file unloaded)))))
            files)))

(defun romacs-lint--source-directories (root)
  "The Lisp directories of the tree at ROOT that stand for `load-path''s:
each directory this machine loads from under a lisp/, where ROOT has it."
  (delete-dups
   (cl-loop for directory in load-path
            for mine = (and (string-match "/lisp/\\(.*\\)\\'" (directory-file-name directory))
                            (expand-file-name (concat "lisp/" (match-string 1 (directory-file-name directory))) root))
            when (and mine (file-directory-p mine)) collect mine)))

(defun romacs-lint--print (results)
  "Print RESULTS, one finding a line and a count; return the count that fails."
  (let ((counts (list (cons 'error 0) (cons 'warning 0) (cons 'hint 0))))
    (pcase-dolist (`(,file . ,findings) results)
      (pcase-dolist (`(,line ,level ,text) findings)
        (cl-incf (alist-get level counts))
        (princ (format "%s:%d: %s: %s\n" (file-relative-name file) line level text))))
    (princ (format "romacs-lint: %d file%s, %d error%s, %d warning%s, %d hint%s\n"
                   (length results) (if (= (length results) 1) "" "s")
                   (alist-get 'error counts) (if (= (alist-get 'error counts) 1) "" "s")
                   (alist-get 'warning counts) (if (= (alist-get 'warning counts) 1) "" "s")
                   (alist-get 'hint counts) (if (= (alist-get 'hint counts) 1) "" "s")))
    (+ (alist-get 'error counts) (alist-get 'warning counts))))

(defun romacs-lint-batch ()
  "Lint the files named on the command line and exit: 1 when any finding
is an error or a warning.  Before the files, `--base REVISION' names the
git revision struct layouts are held to, and `--root DIRECTORY' the
source tree of files kept outside it."
  (let (base)
    (while (member (car command-line-args-left) '("--base" "--root"))
      (pcase (pop command-line-args-left)
        ("--base" (setq base (pop command-line-args-left)))
        ("--root" (setq romacs-lint-root (file-name-as-directory (expand-file-name (pop command-line-args-left)))))))
    (let ((files command-line-args-left))
      (setq command-line-args-left nil)
      (kill-emacs (if (> (romacs-lint--print (romacs-lint-files files base)) 0) 1 0)))))

(provide 'romacs-lint)
;;; romacs-lint.el ends here
