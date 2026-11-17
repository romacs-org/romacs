;;; romacs-treesit.el --- Tree-sitter around visiting a file  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A visit's first parse starts as soon as the file's text is in, on a
;; thread of its own, and runs while `after-find-file' chooses and starts
;; the major mode.  The mode's parser takes the tree at its first parse, so
;; the parse costs only what the mode did not cover.  The language is a
;; guess from `auto-mode-alist', which a file's local variables or magic
;; can overrule: a wrong one only costs the thread its work.
;;
;; Until the tree lands, redisplay shows the text with stopgap faces (see
;; below) and keys run: its landing brings the tree's faces
;; (`treesit_parse_pending' in display.rs).  What
;; runs by itself must not ask for the tree meanwhile, or it waits for the
;; parse with the keys behind it.  Two things do: tree-sitter's own
;; `pre-redisplay-functions' entry, which the frame the landing wakes runs
;; with every change since, and `show-paren-mode's idle timer, which
;; highlights no paren until the tree is there.

;; A typed character does not wait for its parse either.  A frame shows an
;; edit of `romacs-treesit-show-before-parse' bytes or fewer as the text
;; stands, with the faces the character inherited, and the next frame
;; parses and fontifies it: a parse of a large function takes longer than a
;; display's frame (`defer_parse' in display.rs).

;;; Code:

(defcustom romacs-treesit-show-before-parse 8
  "Largest edit, in bytes, a frame shows before the parse it needs.
Typing shows at once and takes its faces a frame later.  A larger edit,
such as a yank, waits for its parse, so that no plain text flashes.  Zero
waits for every parse, as GNU does."
  :type 'integer
  :group 'treesit)

(defconst romacs-treesit--mode-languages
  '((bash-ts-mode . bash) (c-ts-mode . c) (c++-ts-mode . cpp)
    (c-or-c++-ts-mode . c) (css-ts-mode . css) (go-ts-mode . go)
    (js-ts-mode . javascript) (json-ts-mode . json)
    (mhtml-ts-mode . html) (python-ts-mode . python)
    (rust-ts-mode . rust) (toml-ts-mode . toml) (tsx-ts-mode . tsx)
    (typescript-ts-mode . typescript) (yaml-ts-mode . yaml))
  "The language each tree-sitter mode's primary parser reads.")

(defconst romacs-treesit--early-minimum 4096
  "The fewest bytes worth a thread: a smaller file parses in less time.")

(defun romacs-treesit--file-language ()
  "The language the visited file's mode will parse, by its name, or nil."
  (let ((mode (assoc-default buffer-file-name auto-mode-alist #'string-match)))
    (when (consp mode) (setq mode (car mode)))
    (when mode (setq mode (major-mode-remap mode)))
    (when (and mode (symbolp mode))
      (alist-get (intern (string-remove-suffix "-maybe" (symbol-name mode)))
                 romacs-treesit--mode-languages))))

(defun romacs-treesit--visit (visit &rest args)
  "Around `after-find-file': call VISIT with ARGS, the parse running.
A parse no parser came for is abandoned, its thread left to end."
  (let* ((buffer (current-buffer))
         (language (and buffer-file-name enable-multibyte-characters
                        (>= (buffer-size) romacs-treesit--early-minimum)
                        (not (nth 4 args)) ; NOMODES
                        (romacs-treesit--file-language)))
         (started (and language (romacs--treesit-parse-early language))))
    (unwind-protect
        (apply visit args)
      (when (and started (buffer-live-p buffer))
        (with-current-buffer buffer
          (when (and enable-multibyte-characters
                     (not (treesit-parser-list nil nil t)))
            (romacs--treesit-parse-early nil)))))))

;;; Stopgap faces

;; A window whose buffer's first parse has not landed shows lexical faces
;; meanwhile: strings and comments as the syntax table finds them, and
;; the keyword tokens the mode's enabled font-lock queries capture by
;; their text.  Nothing of it asks for the tree.  The landing's
;; fontification replaces it, so the only change it makes to these faces
;; is the structural ones it adds (names, types).

(defvar-local romacs-treesit--stopgap-rules 'unknown
  "The current buffer's stopgap: (DOC-REGEXP . KEYWORDS), KEYWORDS a list
of (REGEXP . FACE); `unknown' until the first stopgap reads the mode's
font-lock settings.")

(defvar-local romacs-treesit--stopgapped nil
  "The (START . END) the last stopgap covered, which the next skips.")

(defun romacs-treesit--stopgap-patterns (source)
  "The keyword tokens and doc-comment regexp of query SOURCE, a sexp.
Each a string or vector of strings captured by a face at the top of a
pattern list; a doc regexp is the `:match' of a `font-lock-doc-face'
comment pattern."
  (let (tokens doc)
    (when (listp source)
      (while source
        (let ((item (pop source)) (capture (car source)))
          (cond
           ((and (or (stringp item) (vectorp item)) (symbolp capture)
                 (string-prefix-p "@" (symbol-name capture)))
            (pop source)
            (let ((face (intern (substring (symbol-name capture) 1))))
              (when (facep face)
                (dolist (token (if (stringp item) (list item) (append item nil)))
                  (when (stringp token) (push (cons token face) tokens))))))
           ((and (consp item) (equal (car item) '(comment))
                 (eq (nth 1 item) '@font-lock-doc-face))
            (let ((match (assq :match (seq-filter #'consp (cddr item)))))
              (when (stringp (nth 1 match)) (setq doc (nth 1 match)))))))))
    (cons doc tokens)))

(defun romacs-treesit--stopgap-read-rules ()
  "The current buffer's stopgap rules, from its enabled font-lock settings."
  (let (doc tokens)
    (dolist (setting (bound-and-true-p treesit-font-lock-settings))
      (when (nth 1 setting)
        (pcase-let ((`(,d . ,ts) (romacs-treesit--stopgap-patterns
                                  (treesit-query-source (nth 0 setting)))))
          (setq doc (or doc d) tokens (append ts tokens)))))
    (cons doc
          (mapcar (lambda (entry)
                    (cons (romacs-treesit--tokens-regexp (cdr entry)) (car entry)))
                  (romacs-treesit--group-by #'cdr #'car tokens)))))

(defun romacs-treesit--group-by (key value items)
  "ITEMS grouped as an alist of (KEY . VALUES), VALUE of each item kept.
Keys compare with `equal'."
  (let (groups)
    (dolist (item items)
      (let* ((k (funcall key item)) (group (assoc k groups)))
        (if group
            (setcdr group (cons (funcall value item) (cdr group)))
          (push (list k (funcall value item)) groups))))
    groups))

(defun romacs-treesit--tokens-regexp (tokens)
  "A regexp for any of TOKENS as a whole token: a symbol boundary at each
end of a token that is a word character there."
  (mapconcat (lambda (group)
               (concat (and (car (car group)) "\\_<") (regexp-opt (cdr group) t)
                       (and (cdr (car group)) "\\_>")))
             (romacs-treesit--group-by
              (lambda (token) (cons (string-match-p "\\`\\w" token) (string-match-p "\\w\\'" token)))
              #'identity tokens)
             "\\|"))

(defun romacs-treesit--stopgap (window start end)
  "Stopgap faces from START to END of WINDOW's buffer, its first parse out."
  (with-current-buffer (window-buffer window)
    (unless (and romacs-treesit--stopgapped
                 (<= (car romacs-treesit--stopgapped) start)
                 (>= (cdr romacs-treesit--stopgapped) end))
      (when (eq romacs-treesit--stopgap-rules 'unknown)
        (setq romacs-treesit--stopgap-rules (romacs-treesit--stopgap-read-rules)))
      (setq romacs-treesit--stopgapped (cons start end))
      ;; No property lookup either: a scan that reaches the propertized
      ;; frontier runs `syntax-propertize' from inside `parse-partial-sexp'.
      (let ((doc (car romacs-treesit--stopgap-rules)) (case-fold-search nil)
            (parse-sexp-lookup-properties nil))
        (with-silent-modifications
          (save-excursion
            (save-match-data
              (remove-list-of-text-properties start end '(face))
              ;; From the text's start, never `syntax-ppss': its
              ;; `syntax-propertize' may ask the tree (c++-ts-mode's
              ;; angle brackets), and its cache would keep states read
              ;; without the properties.
              (let ((state (parse-partial-sexp (point-min) start)) (pos start))
                (while (< pos end)
                  (let ((from (nth 8 state)) (string (nth 3 state)))
                    (setq state (parse-partial-sexp pos end nil nil state 'syntax-table)
                          pos (point))
                    (when from
                      (put-text-property
                       (max start from) pos 'face
                       (cond (string 'font-lock-string-face)
                             ((and doc (save-excursion (goto-char from) (looking-at-p (string-remove-prefix "\\`" doc))))
                              'font-lock-doc-face)
                             (t 'font-lock-comment-face)))))))
              (pcase-dolist (`(,regexp . ,face) (cdr romacs-treesit--stopgap-rules))
                (goto-char start)
                (while (re-search-forward regexp end t)
                  (unless (get-text-property (match-beginning 0) 'face)
                    (put-text-property (match-beginning 0) (match-end 0) 'face face)))))))))))

(defun romacs-treesit--no-parse-p (&rest _)
  "Whether the current buffer's tree is there to ask for."
  (not (romacs--treesit-parsing-p)))

(when (fboundp 'advice-add)
  (advice-add 'after-find-file :around #'romacs-treesit--visit)
  (with-eval-after-load 'treesit
    (advice-add 'treesit--pre-redisplay :before-while #'romacs-treesit--no-parse-p)
    (advice-add 'treesit-show-paren-data :before-while #'romacs-treesit--no-parse-p)))

(provide 'romacs-treesit)
;;; romacs-treesit.el ends here
