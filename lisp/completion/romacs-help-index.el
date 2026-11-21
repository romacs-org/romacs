;;; romacs-help-index.el --- An index of the definitions help does not see yet  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Help's completion (romacs-help-completion.el) loads no file, so a
;; definition in a library not loaded yet is no symbol it can list.  This
;; index names them: every definition of the libraries
;; `definition-prefixes' names, by kind, read from their sources without
;; loading them.  A worker VM makes it (`romacs-help-index-build'), so the
;; main VM never waits: the first help listing asks for it
;; (`romacs-help-index-request') and lists without it until it arrives.
;; The worker keeps it in a file under the user's cache directory, keyed
;; by each source's name, size and modification time, and reads it from
;; there while no source changed; a library registered since, or a
;; `load-path' that finds a source elsewhere, asks for it again.

;;; Code:

(declare-function isolate-spawn "isolate" (function &rest args))
(declare-function isolate-on-done "isolate" (future callback))
(declare-function isolate-await "isolate" (future &optional timeout))
(declare-function isolate-main-p "isolate" ())

;;;; Reading definitions (in the worker)

(defconst romacs-help-index--definition
  "^(\\(\\(?:cl-\\)?def[^ \t\n]+\\)[ \t\n]+['(]*\\([^' ()\"\n]+\\)[\n \t]"
  "A top-level definition's form and name, as loaddefs-gen.el finds them
for prefixes, and also `cl-' definers.")

(defconst romacs-help-index--kinds
  '(("defun" function) ("defmacro" function) ("defsubst" function) ("define-inline" function)
    ("defalias" function) ("cl-defun" function) ("cl-defmacro" function) ("cl-defsubst" function)
    ("cl-defgeneric" function) ("cl-defmethod" function)
    ("define-derived-mode" function command) ("define-generic-mode" function command)
    ("define-compilation-mode" function command) ("define-skeleton" function command)
    ("transient-define-prefix" function command)
    ("define-minor-mode" function command variable)
    ("define-globalized-minor-mode" function command variable)
    ("defvar" variable) ("defcustom" variable) ("defconst" variable) ("defvar-local" variable)
    ("defvaralias" variable) ("defvar-keymap" variable) ("defface" face))
  "What a definer defines its name as.  Other definers are not indexed.")

(defconst romacs-help-index--maybe-command '("defun" "cl-defun")
  "Definers whose definition is a command when its body is interactive.")

(defun romacs-help-index--shorthands ()
  "The `read-symbol-shorthands' of the source in this buffer, longest
shorthand first, as its Local Variables give them."
  (save-excursion
    (goto-char (point-max))
    (when (and (search-backward "Local Variables:" (max (point-min) (- (point-max) 3000)) t)
               (re-search-forward "read-symbol-shorthands: *" nil t))
      (let ((shorthands (ignore-errors (read (current-buffer)))))
        (sort (seq-filter (lambda (s) (and (stringp (car-safe s)) (stringp (cdr-safe s)))) shorthands)
              (lambda (a b) (> (length (car a)) (length (car b)))))))))

(defun romacs-help-index--name (name shorthands)
  "NAME as the reader interns it under SHORTHANDS, decoded as UTF-8."
  (let ((name (if (string-match-p "[^[:ascii:]]" name) (decode-coding-string name 'utf-8) name)))
    (if-let* ((shorthand (seq-find (lambda (s) (string-prefix-p (car s) name)) shorthands)))
        (concat (cdr shorthand) (substring name (length (car shorthand))))
      name)))

(defun romacs-help-index--interactive-p (start)
  "Whether the definition at START has an interactive spec of its own:
its body's first form after the docstring and declarations.  Walked as
text, so nothing is interned; only where an interactive form appears
before the next top-level form."
  (save-excursion
    (let ((end (save-excursion (if (re-search-forward "^(" nil t) (point) (point-max)))))
      (and (re-search-forward "(interactive\\_>" end t)
           (with-syntax-table emacs-lisp-mode-syntax-table
             (ignore-errors
               (goto-char start)
               (down-list 1)
               (forward-sexp 3)              ; The definer, the name, the arguments.
               (forward-comment (buffer-size))
               (when (looking-at "\"")
                 (forward-sexp 1)
                 (forward-comment (buffer-size)))
               (while (looking-at "(declare\\_>")
                 (forward-sexp 1)
                 (forward-comment (buffer-size)))
               (looking-at "(interactive\\_>")))))))

(defun romacs-help-index--scan (library file index)
  "Put LIBRARY's definitions into INDEX, reading its source FILE as bytes.
An entry is ((KIND . LIBRARY) ...), KIND `function', `command',
`variable' or `face'."
  (erase-buffer)
  (insert-file-contents-literally file)
  (let ((shorthands (romacs-help-index--shorthands)))
    (goto-char (point-min))
    (while (re-search-forward romacs-help-index--definition nil t)
      (let* ((definer (match-string-no-properties 1))
             (kinds (cdr (assoc definer romacs-help-index--kinds))))
        (when kinds
          (let* ((name (romacs-help-index--name (match-string-no-properties 2) shorthands))
                 (kinds (if (and (member definer romacs-help-index--maybe-command)
                                 (romacs-help-index--interactive-p (match-beginning 0)))
                            (cons 'command kinds)
                          kinds))
                 (entry (gethash name index)))
            (dolist (kind kinds)
              (unless (assq kind entry) (push (cons kind library) entry)))
            (puthash name entry index)))))))

(defun romacs-help-index--sources (libraries path)
  "An alist of each of LIBRARIES (a list) with the source PATH finds for
it, as `locate-library' finds a `.el' file on `load-path': the first
directory holding it, nil standing for `default-directory'.  Each
directory is listed once; a library named with a directory, as
semantic/analyze, is looked for in each.  A library with no source is
left out."
  (let ((dirs (mapcar (lambda (dir) (if dir (expand-file-name dir) default-directory)) path))
        (top (make-hash-table :test 'equal)) (sources nil))
    (dolist (dir dirs)
      (when (and (stringp dir) (file-directory-p dir))
        (dolist (file (directory-files dir t "\\.el\\'" t))
          (let ((library (file-name-sans-extension (file-name-nondirectory file))))
            (unless (gethash library top) (puthash library file top))))))
    (dolist (library libraries)
      (when-let* ((file (if (not (string-search "/" library)) (gethash library top)
                          (seq-some (lambda (dir)
                                      (let ((file (expand-file-name (concat library ".el") dir)))
                                        (and (file-exists-p file) file)))
                                    dirs))))
        (push (cons library file) sources)))
    (nreverse sources)))

(defun romacs-help-index--key (sources)
  "What SOURCES' index is made of: each source's name, size and
modification time, digested."
  (secure-hash 'sha256
               (prin1-to-string
                (mapcar (lambda (source)
                          (let ((attributes (file-attributes (cdr source) 'integer)))
                            (list (car source) (cdr source)
                                  (file-attribute-size attributes)
                                  (file-attribute-modification-time attributes))))
                        sources))))

(defun romacs-help-index--read-cache (file key)
  "The index FILE holds for KEY, or nil."
  (when (file-readable-p file)
    (ignore-errors
      (with-temp-buffer
        (insert-file-contents file)
        (let ((cached (read (current-buffer))))
          (and (equal (car-safe cached) key) (hash-table-p (cdr cached)) (cdr cached)))))))

(defun romacs-help-index--write-cache (file key index)
  "Keep INDEX for KEY in FILE, replacing it whole."
  (ignore-errors
    (make-directory (file-name-directory file) t)
    (let ((temp (make-temp-file (expand-file-name "help-index-" (file-name-directory file))))
          (print-length nil) (print-level nil) (print-circle nil))
      (with-temp-file temp
        (set-buffer-multibyte t)
        (let ((coding-system-for-write 'utf-8-unix))
          (prin1 (cons key index) (current-buffer))))
      (rename-file temp file t))))

(defun romacs-help-index-build (libraries path cache)
  "The index of LIBRARIES' definitions, their sources found on PATH (a
`load-path'): a hash table from a name to ((KIND . LIBRARY) ...).  Read
from the file CACHE while its key holds, else read from the sources and
kept there.  Runs in a worker (`romacs-help-index-request')."
  (let* ((sources (romacs-help-index--sources libraries path))
         (key (romacs-help-index--key sources)))
    (or (romacs-help-index--read-cache cache key)
        (let ((index (make-hash-table :test 'equal :size 100000)))
          (with-temp-buffer
            (set-buffer-multibyte nil)
            (dolist (source sources)
              (romacs-help-index--scan (car source) (cdr source) index)))
          (romacs-help-index--write-cache cache key index)
          index))))

;;;; The index in the main VM

(defvar romacs-help-index nil
  "The index of definitions (`romacs-help-index-build'), once a worker
made it, else nil.")

(defvar romacs-help-index--asked nil
  "What the index, or the job making it, was asked for: (LIBRARIES PATH).")

(defvar romacs-help-index--future nil
  "The job making the index, while it runs.")

(defvar romacs-help-index-last-error nil
  "The last error a job making the index ended with.")

(defun romacs-help-index--libraries ()
  "The libraries `definition-prefixes' names, each once."
  (let ((seen (make-hash-table :test 'equal)) (libraries nil))
    (maphash (lambda (_prefix names)
               (dolist (name names)
                 (unless (gethash name seen) (puthash name t seen) (push name libraries))))
             definition-prefixes)
    libraries))

(defun romacs-help-index--cache-file ()
  "Where the worker keeps the index: under the user's cache directory."
  (expand-file-name "romacs/help-index.eld"
                    (or (getenv "XDG_CACHE_HOME") (expand-file-name "~/.cache"))))

(defun romacs-help-index-request ()
  "Have a worker make the index, unless it or a job making it is for the
libraries and `load-path' of now.  Returns at once.  A batch session
makes none."
  (let* ((libraries (romacs-help-index--libraries))
         (asked (list (length libraries) load-path)))
    (unless (or noninteractive (equal asked romacs-help-index--asked)
                (not (fboundp 'isolate-spawn)) (not (isolate-main-p)))
      (setq romacs-help-index--asked asked)
      (condition-case err
          (let ((future (isolate-spawn 'romacs-help-index-build libraries load-path
                                       (romacs-help-index--cache-file)
                                       :name "help-index" :require '(romacs-help-index))))
            (setq romacs-help-index--future future)
            (isolate-on-done future #'romacs-help-index--done))
        (error (setq romacs-help-index-last-error err romacs-help-index--asked nil))))))

(defun romacs-help-index--done (future)
  "FUTURE, a job making the index, has resolved: take its index."
  (when (eq future romacs-help-index--future)
    (setq romacs-help-index--future nil)
    (condition-case err
        (setq romacs-help-index (isolate-await future 0))
      ;; A later listing asks again.
      ((error quit) (setq romacs-help-index-last-error err romacs-help-index--asked nil)))))

(provide 'romacs-help-index)
;;; romacs-help-index.el ends here
