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
(require 'romacs-theme)

(defgroup romacs-lexical nil
  "Code coloured by its lexical classes alone."
  :group 'faces
  :prefix "romacs-lexical-")

(defface romacs-lexical-head '((t))
  "The symbol a Lisp list starts with when it calls a function: the
text's own colour by default, a call being no more than code.")
(defface romacs-lexical-added '((t :inherit diff-added :extend t))
  "A diff's added line, to the window's edge.")
(defface romacs-lexical-removed '((t :inherit diff-removed :extend t))
  "A diff's removed line, to the window's edge.")
(defface romacs-lexical-hunk '((t :inherit diff-header :extend t))
  "A diff's hunk header: a band, never a line's colour.")
(defface romacs-lexical-added-sign '((t :inherit (diff-indicator-added romacs-lexical-added)))
  "An added line's `+', in the theme's colour for additions, on its band.")
(defface romacs-lexical-removed-sign '((t :inherit (diff-indicator-removed romacs-lexical-removed)))
  "A removed line's `-', in the theme's colour for removals, on its band.")

(defun romacs-lexical--derive (&rest _)
  "Band a hunk header in the theme's `diff-header' background where it
stands 1.15:1 apart from the text's, else in the text's background mixed
toward its colour as far (`romacs-color-band'): a band on every theme,
its colour still the header's.  Make a diff line's sign its indicator's
colour, legible on the line's band.  Each as the face's default spec,
so a theme's own spec wins."
  (let* ((bg (romacs-color-hex (face-background 'default nil t)))
         (fg (romacs-color-hex (face-foreground 'default nil t)))
         (theirs (and (facep 'diff-header) (romacs-color-hex (face-background 'diff-header nil t))))
         (band (cond ((and theirs bg (>= (romacs-color-contrast theirs bg) 1.15)) theirs)
                     ((and bg fg) (romacs-color-band bg fg)))))
    (face-spec-set 'romacs-lexical-hunk
                   `((t :inherit diff-header ,@(and band `(:background ,band)) :extend t))
                   'face-defface-spec))
  ;; Each sign in its indicator's colour raised to 4.5:1 on its line's band.
  (pcase-dolist (`(,sign ,indicator ,line) '((romacs-lexical-added-sign diff-indicator-added diff-added)
                                             (romacs-lexical-removed-sign diff-indicator-removed diff-removed)))
    (let ((colour (and (facep indicator)
                       (romacs-color-legible (face-foreground indicator nil t)
                                             (or (face-background line nil t) (face-background 'default nil t))))))
      (face-spec-set sign `((t :inherit (,indicator ,(if (eq sign 'romacs-lexical-added-sign) 'romacs-lexical-added 'romacs-lexical-removed))
                               ,@(and colour `(:foreground ,colour))))
                     'face-defface-spec))))

(add-hook 'enable-theme-functions #'romacs-lexical--derive)
(add-hook 'disable-theme-functions #'romacs-lexical--derive)
(romacs-lexical--derive)

(defcustom romacs-lexical-faces
  '((comment . font-lock-comment-face) (string . font-lock-string-face)
    (number . font-lock-number-face) (keyword . font-lock-builtin-face)
    (char . font-lock-constant-face) (head . romacs-lexical-head) (form . font-lock-keyword-face)
    (name . font-lock-function-name-face)
    (added . romacs-lexical-added) (removed . romacs-lexical-removed) (hunk . romacs-lexical-hunk)
    (added-sign . romacs-lexical-added-sign) (removed-sign . romacs-lexical-removed-sign))
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
    (diff . diff) (patch . diff) (shell . hash) (console . hash)
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
                      ((memq char '(?: ?&)) 'keyword)
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
which holds to its end, an added or removed line's sign a class of its
own; its newline starts the next line afresh."
  (cond ((= char ?\n) 'bol)
        ((eq state 'bol) (pcase char (?+ 'added-sign) (?- 'removed-sign) (?@ 'hunk) (_ 'context)))
        ((eq state 'added-sign) 'added)
        ((eq state 'removed-sign) 'removed)
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
    ((or 'added 'removed 'hunk 'added-sign 'removed-sign) state)
    (_ nil)))

;;; Colouring

(defun romacs-lexical-family (language)
  "The lexical family LANGUAGE, a symbol, is coloured as, or nil."
  (alist-get language romacs-lexical-languages))

(defun romacs-lexical--span (state before at next end)
  "The span (FROM TO CLASS) of a run lexed in STATE from AT to NEXT, the
run before it in BEFORE, END the region's: nil for plain code."
  (let* ((class (romacs-lexical--class state))
         ;; A comment's opening slash was lexed before it knew.
         (at (if (and (eq class 'comment) (eq before 'slash)) (1- at) at)))
    (pcase class
      ('nil nil)
      ;; A head that is a form, not a call.
      ('head (list at next (if (string-match-p romacs-lexical-lisp-forms (buffer-substring-no-properties at next)) 'form 'head)))
      ;; A diff's line is coloured through its newline, a band.
      ((or 'added 'removed 'hunk 'added-sign 'removed-sign)
       (list at (if (and (< next end) (eq (char-after next) ?\n)) (1+ next) next) class))
      (_ (list at next class)))))

(defun romacs-lexical--defined-name (span)
  "The name a definition's head, SPAN, defines, as a span, or nil."
  (pcase-let ((`(,from ,to ,_) span))
    (when (string-match-p "\\`\\(?:cl-\\)?def" (buffer-substring-no-properties from to))
      (save-excursion
        (goto-char to)
        (when (looking-at "[ \t\n]+\\([^] \t\n()\"';]+\\)")
          (list (match-beginning 1) (match-end 1) 'name))))))

(defun romacs-lexical--joined (span spans)
  "SPANS, the newest first, with SPAN after them: one span with the last
when it goes on in the same class (a string's escapes, a char's)."
  (pcase-let ((`(,from ,to ,class) span)
              (`(,last-from ,last-to ,last-class) (car spans)))
    (if (and spans (eq last-class class) (= last-to from))
        (cons (list last-from to class) (cdr spans))
      (cons span spans))))

(defun romacs-lexical-spans (family beg end)
  "The classes of BEG..END of the current buffer lexed as FAMILY, as
\(FROM TO CLASS) spans in order, plain code left out.  After a
definition's head, its name; its list of arguments calls nothing."
  (let ((runs (romacs-scanner-runs (alist-get family romacs-lexical--families) beg end))
        (before nil) (spans nil) (defining nil))
    (while runs
      (pcase-let* ((`(,at . ,state) (car runs))
                   (span (romacs-lexical--span state before at (or (car-safe (cadr runs)) end) end)))
        (when (and defining span (eq (nth 2 span) 'head)) (setq span nil defining nil))
        (when span
          (setq spans (romacs-lexical--joined span spans))
          (when-let* ((name (and (eq (nth 2 span) 'form) (romacs-lexical--defined-name span))))
            (push name spans)
            (setq defining t)))
        (setq before state runs (cdr runs))))
    (nreverse spans)))

(defun romacs-lexical-colour (language beg end)
  "Colour BEG..END of the current buffer, LANGUAGE's code, by its lexical
classes, each face in front of those there.  Return non-nil when
LANGUAGE has a family (`romacs-lexical-languages'), else do nothing."
  (when-let* ((family (romacs-lexical-family language)))
    ;; Its faces, the hunk's band then derived from them.
    (when (and (eq family 'diff) (not (featurep 'diff-mode)))
      (require 'diff-mode)
      (romacs-lexical--derive))
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
