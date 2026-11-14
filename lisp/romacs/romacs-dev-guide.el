;;; romacs-dev-guide.el --- What this Romacs offers, asked of itself  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A guide to writing Lisp for this machine that is read from the machine:
;; nothing here is a list of what exists.
;;
;;   (romacs-dev-guide "isolate-spawn")   what one function is: its kind and
;;       arguments, a primitive's effect class and Rust file, its
;;       docstring, and what callers are told of it (`romacs-lint-hint').
;;   (romacs-dev-guide "worker snapshot") the functions whose name or
;;       docstring has every word, names first.
;;   (romacs-dev-guide "")                the better paths their owners
;;       declared, and how many primitives each effect class has.
;;
;; A docstring is the vocabulary: a function is found by what its docstring
;; says, so a better docstring is a better guide.  The search reads every
;; docstring of the machine it is asked in, the functions defined since it
;; started among them, which takes seconds.  So it can be taken in steps:
;; `romacs-dev-guide-search-start', `romacs-dev-guide-search-step' for a
;; few milliseconds at a time, `romacs-dev-guide-search-result'.  The MCP
;; tool romacs_dev_guide steps it from a timer, and the command loop runs
;; between the steps.

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(defconst romacs-dev-guide--effects
  '((pure . "depends on its arguments alone")
    (reads . "reads the machine's state, writes none")
    (mutates-args . "writes what it is given, nothing else")
    (global . "writes the machine's state"))
  "What each effect class of a primitive means (`romacs--subr-effect').")

(defun romacs-dev-guide--effect (symbol)
  "The effect class of primitive SYMBOL, or nil."
  (let ((definition (indirect-function symbol)))
    (and (subrp definition) (fboundp 'romacs--subr-effect)
         (funcall 'romacs--subr-effect definition))))

(defun romacs-dev-guide--kind (symbol)
  "What the function SYMBOL is, in a few words."
  (let ((definition (indirect-function symbol))
        (effect (romacs-dev-guide--effect symbol)))
    (cond ((special-form-p symbol) "special form")
          ((subrp definition) (if effect (format "primitive, %s" effect) "primitive"))
          ((macrop definition) "macro")
          ((autoloadp definition) (format "autoloaded from %s" (nth 1 definition)))
          ((commandp symbol) "command")
          (t "function"))))

(defun romacs-dev-guide--summary (symbol)
  "The first line of the function SYMBOL's docstring, or nil."
  (let ((doc (ignore-errors (documentation symbol t))))
    (and doc (not (string-empty-p doc)) (car (split-string doc "\n")))))

(defun romacs-dev-guide--file (symbol)
  "The file that defines the function SYMBOL, from the source tree, or nil."
  (let ((definition (indirect-function symbol)))
    (if (subrp definition)
        (and (fboundp 'romacs--rust-definition) (car (funcall 'romacs--rust-definition definition nil)))
      (when-let* ((file (symbol-file symbol 'defun)))
        (if (string-match "/\\(\\(?:lisp\\|test\\)/.*\\)\\'" file) (match-string 1 file) file)))))

(defun romacs-dev-guide--arguments (symbol)
  "The argument list of the function SYMBOL as text, or its arity."
  (let ((arguments (ignore-errors (help-function-arglist symbol t))))
    (cond ((listp arguments) (format "%S" arguments))
          ((stringp arguments) arguments)
          (t (format "%S" (ignore-errors (func-arity symbol)))))))

(defun romacs-dev-guide-describe (symbol)
  "What the function SYMBOL is, as lines of text."
  (let* ((doc (ignore-errors (documentation symbol t)))
         (lines (and doc (split-string (string-trim doc) "\n")))
         (effect (romacs-dev-guide--effect symbol))
         (file (romacs-dev-guide--file symbol))
         (alias (and (symbolp (symbol-function symbol)) (symbol-function symbol)))
         (obsolete (get symbol 'byte-obsolete-info))
         (hint (get symbol 'romacs-lint-hint)))
    (string-join
     (delq nil
           (append
            (list (format "%s %s: %s%s" symbol (romacs-dev-guide--arguments symbol) (romacs-dev-guide--kind symbol)
                          (if file (format ", %s" file) ""))
                  (and alias (format "  alias of `%s'" alias))
                  (and effect (format "  %s: %s" effect (alist-get effect romacs-dev-guide--effects)))
                  (and obsolete (format "  obsolete%s%s" (if (nth 2 obsolete) (format " since %s" (nth 2 obsolete)) "")
                                        (let ((new (car obsolete)))
                                          (cond ((stringp new) (concat ": " new)) (new (format ": use `%s'" new)) (t "")))))
                  (and (stringp hint) (format "  callers are told: %s" hint)))
            (mapcar (lambda (line) (concat "  " line)) (seq-take lines 16))
            (and (> (length lines) 16) (list (format "  ... %d more lines: (documentation '%s)" (- (length lines) 16) symbol)))))
     "\n")))

(defun romacs-dev-guide--functions ()
  "Every symbol that names a function here."
  (let (functions)
    (mapatoms (lambda (symbol) (when (fboundp symbol) (push symbol functions))))
    functions))

(defun romacs-dev-guide--hit (symbol words)
  "SYMBOL as a hit when its name or docstring has every one of WORDS:
(SYMBOL WORDS-IN-ITS-NAME INTERNAL), INTERNAL 1 for a name with \"--\"."
  (let* ((name (symbol-name symbol))
         (named (cl-count-if (lambda (word) (string-search word name)) words))
         (case-fold-search t))
    (when (or (= named (length words))
              (let ((doc (ignore-errors (documentation symbol t))))
                (and doc (cl-every (lambda (word) (or (string-search word name) (string-match-p (regexp-quote word) doc)))
                                   words))))
      (list symbol named (if (string-search "--" name) 1 0)))))

(defun romacs-dev-guide-search-start (query &optional functions)
  "A search for the words of the string QUERY, not yet begun: (WORDS REST
HITS).  FUNCTIONS are the symbols searched, default every function here."
  (list (split-string (downcase query) "[ \t\n]+" t) (or functions (romacs-dev-guide--functions)) nil))

(defun romacs-dev-guide-search-step (search &optional seconds)
  "SEARCH taken further, for about SECONDS, or to its end when nil.
A step judges at least one function, so stepping always ends."
  (pcase-let ((`(,words ,rest ,hits) search)
              (end (and seconds (+ (float-time) seconds))))
    (while (progn
             (when-let* ((hit (and rest (romacs-dev-guide--hit (pop rest) words)))) (push hit hits))
             (and rest (or (null end) (< (float-time) end)))))
    (list words rest hits)))

(defun romacs-dev-guide-search-done-p (search)
  "Non-nil when SEARCH has judged every function."
  (null (nth 1 search)))

(defun romacs-dev-guide-search-result (search &optional limit)
  "What SEARCH found, as lines of text, at most LIMIT hits (default 30):
more of its words in a name first, then public before internal, then by
name."
  (let* ((words (car search))
         (hits (mapcar #'car
                       (sort (copy-sequence (nth 2 search))
                             (lambda (a b)
                               (or (> (nth 1 a) (nth 1 b))
                                   (and (= (nth 1 a) (nth 1 b))
                                        (or (< (nth 2 a) (nth 2 b))
                                            (and (= (nth 2 a) (nth 2 b)) (string< (car a) (car b))))))))))
         (limit (or limit 30)))
    (string-join
     (append
      (mapcar (lambda (symbol)
                (format "%s (%s): %s" symbol (romacs-dev-guide--kind symbol)
                        (or (romacs-dev-guide--summary symbol) "no docstring")))
              (seq-take hits limit))
      (list (cond ((null hits) (format "nothing here has %s in its name or docstring" (string-join words " and ")))
                  ((> (length hits) limit) (format "... %d more: add a word, or ask for one by name" (- (length hits) limit)))
                  (t (format "%d function%s; ask for one by name for its arguments and docstring"
                             (length hits) (if (= (length hits) 1) "" "s"))))))
     "\n")))

(defun romacs-dev-guide-search (query &optional limit functions)
  "The functions that match every word of the string QUERY, as lines of
text, at most LIMIT of them (default 30).  A word matches a name or a
docstring.  FUNCTIONS are the symbols searched, default every function
here."
  (romacs-dev-guide-search-result (romacs-dev-guide-search-step (romacs-dev-guide-search-start query functions)) limit))

(defun romacs-dev-guide-paths ()
  "The better paths their owners declared, and the primitives by effect, as text."
  (let (hints (classes nil) (unclassed 0))
    (mapatoms
     (lambda (symbol)
       (when (stringp (get symbol 'romacs-lint-hint))
         (push (format "%s: %s" symbol (get symbol 'romacs-lint-hint)) hints))
       (when (and (fboundp symbol) (subrp (symbol-function symbol)) (not (special-form-p symbol)))
         (let ((effect (romacs-dev-guide--effect symbol)))
           (if effect (cl-incf (alist-get effect classes 0)) (cl-incf unclassed))))))
    (string-join
     (append
      (list "What callers of a function are told (romacs-lint-hint):")
      (or (mapcar (lambda (hint) (concat "  " hint)) (sort hints #'string<)) (list "  nothing is declared"))
      (list "Primitives by effect class (ask for one by name for its class):")
      (mapcar (lambda (class)
                (format "  %s, %d: %s" (car class) (alist-get (car class) classes 0) (cdr class)))
              romacs-dev-guide--effects)
      (list (format "  no class declared: %d" unclassed)
            "Ask with words for the functions whose name or docstring has them all."))
     "\n")))

(defun romacs-dev-guide--function (query)
  "The function the string QUERY names, or nil."
  (let ((symbol (and (not (string-match-p "[ \t\n]" query)) (intern-soft query))))
    (and symbol (fboundp symbol) symbol)))

(defun romacs-dev-guide-search-p (query)
  "Non-nil when the string QUERY is answered by a search of every
docstring, which takes seconds and is better taken in steps."
  (let ((query (string-trim query)))
    (not (or (string-empty-p query) (romacs-dev-guide--function query)))))

(defun romacs-dev-guide (query &optional limit)
  "What this machine offers for QUERY, a string, as text: a function's name
describes it, words search names and docstrings (at most LIMIT lines),
and an empty QUERY gives the declared better paths."
  (let* ((query (string-trim (or query "")))
         (symbol (romacs-dev-guide--function query)))
    (cond ((string-empty-p query) (romacs-dev-guide-paths))
          (symbol (romacs-dev-guide-describe symbol))
          (t (romacs-dev-guide-search query limit)))))

(provide 'romacs-dev-guide)
;;; romacs-dev-guide.el ends here
