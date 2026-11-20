;;; romacs-lexical.el --- Code coloured by its lexical classes alone -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Code in a language with no tree-sitter grammar here (a code block in
;; an agent's reply, a row of Lisp a tool ran) is coloured by what a
;; lexer alone can tell: comments, strings, numbers, keywords, a diff's
;; added and removed lines.  No major mode runs and no buffer is made:
;; each family of languages is a scanner (romacs-scanner.el), a table
;; run natively over the text, which marks only where the class
;; changes, its table there saying the class that starts
;; (`romacs-scanner-runs').  The cost is a table walk plus a lookup a mark.
;;
;; `romacs-lexical-languages' says which family a language is; a
;; family's classes take the faces of `romacs-lexical-faces'.

;;; Code:

(require 'romacs-scanner)

(defgroup romacs-lexical nil
  "Code coloured by its lexical classes alone."
  :group 'faces
  :prefix "romacs-lexical-")

(defface romacs-lexical-head '((t :inherit font-lock-function-call-face))
  "The symbol a Lisp list starts with: the function, macro or form it calls.")
(defface romacs-lexical-added '((t :inherit diff-added))
  "A diff's added line.")
(defface romacs-lexical-removed '((t :inherit diff-removed))
  "A diff's removed line.")
(defface romacs-lexical-hunk '((t :inherit diff-hunk-header))
  "A diff's hunk header, and its file header lines.")

(defcustom romacs-lexical-faces
  '((comment . font-lock-comment-face) (string . font-lock-string-face)
    (number . font-lock-number-face) (keyword . font-lock-builtin-face)
    (char . font-lock-constant-face) (head . romacs-lexical-head) (form . font-lock-keyword-face)
    (added . romacs-lexical-added) (removed . romacs-lexical-removed) (hunk . romacs-lexical-hunk))
  "The face of each lexical class.  A class left out gets no face."
  :type '(alist :key-type symbol :value-type face))

(defcustom romacs-lexical-lisp-forms
  "\\`\\(?:cl-\\)?\\(?:def[a-z-]*\\|lambda\\|let\\*?\\|if\\|when\\|unless\\|cond\\|and\\|or\\|not\\|progn\\|prog1\\|setq\\|setf\\|while\\|dolist\\|dotimes\\|pcase[a-z-]*\\|condition-case\\|unwind-protect\\|catch\\|throw\\|quote\\|function\\|interactive\\|require\\|provide\\|save-[a-z-]+\\|with-[a-z-]+\\|if-let\\*?\\|when-let\\*?\\|and-let\\*\\|define\\|set!\\|begin\\|loop\\|ignore-errors\\)\\'"
  "The heads of Lisp lists coloured as keywords, not as calls: definitions,
special forms and the commonest macros, as a regexp a head matches whole."
  :type 'regexp)

(defcustom romacs-lexical-languages
  '((emacs-lisp . lisp) (lisp . lisp) (common-lisp . lisp) (scheme . lisp) (clojure . lisp)
    (racket . lisp) (fennel . lisp)
    (diff . diff) (patch . diff)
    (makefile . hash) (make . hash) (conf . hash) (ini . hash) (dockerfile . hash) (nix . hash)
    (perl . hash) (r . hash) (fish . hash) (nim . hash) (elixir . hash) (julia . hash)
    (zig . slash) (kotlin . slash) (swift . slash) (scala . slash) (dart . slash) (groovy . slash)
    (php . slash) (protobuf . slash) (proto . slash) (glsl . slash) (wgsl . slash) (solidity . slash))
  "Languages with no tree-sitter grammar here, to the lexical family that
colours them: `lisp' (; comments, \"strings\", ?chars, :keywords, and
the symbol a list starts with),
`hash' (# comments, quoted strings), `slash' (// and /* */ comments,
quoted strings) or `diff' (added, removed and header lines).  Numbers
are coloured in every family but `diff'."
  :type '(alist :key-type symbol :value-type (choice (const lisp) (const hash) (const slash) (const diff))))

;;; The families' step functions: STATE and CHAR to the state after CHAR.

(defun romacs-lexical--word-char-p (char)
  "Whether CHAR continues a word or symbol, so a digit in it is no number."
  (or (and (>= char ?a) (<= char ?z)) (and (>= char ?A) (<= char ?Z))
      (and (>= char ?0) (<= char ?9)) (memq char '(?_ ?- ?$))
      (= char romacs-scanner-other)))

(defun romacs-lexical--code (char string-quotes)
  "The state CHAR starts in code: a string at one of STRING-QUOTES, a
number at a digit, else a word or plain code."
  (cond ((memq char string-quotes) (cons 'string char))
        ((and (>= char ?0) (<= char ?9)) 'number)
        ((romacs-lexical--word-char-p char) 'word)
        (t 'code)))

(defun romacs-lexical--string (state char)
  "The state after CHAR inside STATE, a string, (string . QUOTE) or its
escape (escape . QUOTE); its closing quote ends it as `string-end'."
  (pcase state
    (`(escape . ,quote) (cons 'string quote))
    (`(string . ,quote) (cond ((= char ?\\) (cons 'escape quote))
                              ((= char quote) 'string-end)
                              (t state)))))

(defun romacs-lexical--number (char)
  "The state after CHAR inside a number: a number goes on through its
digits, letters, dots and underscores (hex, exponents, suffixes)."
  (if (or (romacs-lexical--word-char-p char) (= char ?.)) 'number 'code))

(defun romacs-lexical--lisp (state char)
  "The `lisp' family's step."
  (let ((code (lambda (char)
                (cond ((= char ?\;) 'comment)
                      ((= char ?\() 'open)
                      ((= char ??) 'char)
                      ((= char ?:) 'keyword)
                      (t (romacs-lexical--code char '(?\")))))))
    (pcase state
      ((or 'code 'string-end) (funcall code char))
      ;; A list's first symbol: what it calls.
      ('open (if (and (romacs-lexical--word-char-p char) (not (and (>= char ?0) (<= char ?9)))) 'head (funcall code char)))
      ('head (if (or (romacs-lexical--word-char-p char) (memq char '(?: ?? ?* ?+ ?/ ?< ?> ?= ?! ?%))) 'head (funcall code char)))
      ('word (if (or (romacs-lexical--word-char-p char) (memq char '(?: ?? ?* ?+ ?/ ?< ?> ?= ?! ?%))) 'word (funcall code char)))
      ('number (if (or (romacs-lexical--word-char-p char) (= char ?.)) 'number (funcall code char)))
      ('comment (if (= char ?\n) 'code 'comment))
      ;; ?x, ?\x: the character and its escape.
      ('char (if (= char ?\\) 'char-escape 'char-end))
      ('char-escape 'char-end)
      ('char-end (funcall code char))
      ('keyword (if (or (romacs-lexical--word-char-p char) (memq char '(?: ?* ?+ ?/ ?< ?> ?= ?!))) 'keyword (funcall code char)))
      (_ (romacs-lexical--string state char)))))

(defun romacs-lexical--hash (state char)
  "The `hash' family's step."
  (let ((code (lambda (char) (if (= char ?#) 'comment (romacs-lexical--code char '(?\" ?'))))))
    (pcase state
      ((or 'code 'string-end) (funcall code char))
      ('word (if (romacs-lexical--word-char-p char) 'word (funcall code char)))
      ('number (if (eq (romacs-lexical--number char) 'number) 'number (funcall code char)))
      ('comment (if (= char ?\n) 'code 'comment))
      (_ (romacs-lexical--string state char)))))

(defun romacs-lexical--slash (state char)
  "The `slash' family's step: a / that starts no comment is code."
  (let ((code (lambda (char) (if (= char ?/) 'slash (romacs-lexical--code char '(?\" ?'))))))
    (pcase state
      ((or 'code 'string-end) (funcall code char))
      ('word (if (romacs-lexical--word-char-p char) 'word (funcall code char)))
      ('number (if (eq (romacs-lexical--number char) 'number) 'number (funcall code char)))
      ('slash (cond ((= char ?/) 'line-comment) ((= char ?*) 'block-comment) (t (funcall code char))))
      ('line-comment (if (= char ?\n) 'code 'line-comment))
      ('block-comment (if (= char ?*) 'block-star 'block-comment))
      ('block-star (cond ((= char ?/) 'block-end) ((= char ?*) 'block-star) (t 'block-comment)))
      ('block-end (funcall code char))
      (_ (romacs-lexical--string state char)))))

(defun romacs-lexical--diff (state char)
  "The `diff' family's step: a line's first character says its class,
which holds to its end; its newline starts the next line afresh."
  (cond ((= char ?\n) 'bol)
        ((eq state 'bol) (pcase char (?+ 'added) (?- 'removed) (?@ 'hunk) (_ 'context)))
        (t state)))

(define-scanner romacs-lexical--lisp-scanner (romacs-scanner-marking #'romacs-lexical--lisp) 'code
  "The `lisp' family's lexer.")
(define-scanner romacs-lexical--hash-scanner (romacs-scanner-marking #'romacs-lexical--hash) 'code
  "The `hash' family's lexer.")
(define-scanner romacs-lexical--slash-scanner (romacs-scanner-marking #'romacs-lexical--slash) 'code
  "The `slash' family's lexer.")
(define-scanner romacs-lexical--diff-scanner (romacs-scanner-marking #'romacs-lexical--diff) 'bol
  "The `diff' family's lexer.")

(defconst romacs-lexical--families
  `((lisp . ,romacs-lexical--lisp-scanner) (hash . ,romacs-lexical--hash-scanner)
    (slash . ,romacs-lexical--slash-scanner) (diff . ,romacs-lexical--diff-scanner))
  "Each family's scanner.")

(defun romacs-lexical--class (state)
  "The lexical class of text in STATE, or nil for plain code."
  (pcase state
    ((or 'comment 'line-comment 'block-comment 'block-star 'block-end) 'comment)
    ((or 'string-end `(string . ,_) `(escape . ,_)) 'string)
    ('number 'number)
    ('keyword 'keyword)
    ('head 'head)
    ((or 'char 'char-escape 'char-end) 'char)
    ((or 'added 'removed 'hunk) state)
    (_ nil)))

;;; Colouring

(defun romacs-lexical-family (language)
  "The lexical family LANGUAGE, a symbol, is coloured as, or nil."
  (alist-get language romacs-lexical-languages))

(defun romacs-lexical-spans (family beg end)
  "The classes of BEG..END of the current buffer lexed as FAMILY, as
\(FROM TO CLASS) spans in order, plain code left out."
  (let ((runs (romacs-scanner-runs (alist-get family romacs-lexical--families) beg end))
        (before nil) (spans nil))
    (while runs
      (pcase-let* ((`(,at . ,state) (car runs))
                   (next (or (car-safe (cadr runs)) end))
                   (class (romacs-lexical--class state))
                   ;; A comment's opening slash was lexed before it knew.
                   (at (if (and (eq class 'comment) (eq before 'slash)) (1- at) at))
                   (last (car spans)))
        ;; A head that is a form, not a call.
        (when (and (eq class 'head) (string-match-p romacs-lexical-lisp-forms (buffer-substring-no-properties at next)))
          (setq class 'form))
        (cond ((null class))
              ;; The same class on: one span (a string's escapes, a char's).
              ((and last (eq (nth 2 last) class) (= (nth 1 last) at)) (setcar (cdr last) next))
              (t (push (list at next class) spans)))
        (setq before state runs (cdr runs))))
    (nreverse spans)))

(defun romacs-lexical-colour (language beg end)
  "Colour BEG..END of the current buffer, LANGUAGE's code, by its lexical
classes, each face in front of those there.  Return non-nil when
LANGUAGE has a family (`romacs-lexical-languages'), else do nothing."
  (when-let* ((family (romacs-lexical-family language)))
    (when (eq family 'diff) (require 'diff-mode))
    (pcase-dolist (`(,from ,to ,class) (romacs-lexical-spans family beg end))
      (when-let* ((face (alist-get class romacs-lexical-faces)))
        (add-face-text-property from to face)))
    t))

(defun romacs-lexical-string (language string)
  "A copy of STRING coloured as LANGUAGE's code (`romacs-lexical-colour')."
  (with-temp-buffer
    (insert string)
    (romacs-lexical-colour language (point-min) (point-max))
    (buffer-string)))

(provide 'romacs-lexical)
;;; romacs-lexical.el ends here
