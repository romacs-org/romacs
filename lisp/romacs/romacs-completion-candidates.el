;;; romacs-completion-candidates.el --- Candidate listing for native completion styles  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The completion styles that match natively (`orderless',
;; lisp/romacs/romacs-orderless.el, and `fzf', lisp/romacs/romacs-fzf.el) list a
;; table's candidates here, once, and filter the list in Rust.  A file
;; name table's listing is kept while its directory is unchanged, so a
;; prompt lists a directory once rather than on every key.

;;; Code:

(define-error 'romacs-completion-stale
  "A newer completion pass superseded this one")

(defvar romacs-completion--file-candidates nil
  "A file name table's candidates while its directory is unchanged.
\(KEY CANDIDATES . NAMES), KEY as `romacs-completion--file-key' makes it
and NAMES as `romacs-completion--file-names'.")

(defun romacs-completion--file-key (prefix table pred)
  "What decides TABLE's candidates for PREFIX, when TABLE lists a directory.
It includes the directory's modification time, link count and inode,
so that a change to its entries is a different key.  Nil for any other
TABLE or PRED, and where a file name handler or an environment variable
is involved."
  (when (and (memq table '(read-file-name-internal completion-file-name-table))
             ;; `completion-file-name-table' lists for this PRED as for nil.
             (memq pred '(nil file-exists-p))
             (not (string-search "$" prefix)))
    (let ((dir (expand-file-name (or (file-name-directory (substitute-in-file-name prefix))
                                     default-directory))))
      (unless (find-file-name-handler dir 'file-name-all-completions)
        (when-let* ((attributes (file-attributes dir)))
          (list table prefix default-directory completion-ignore-case
                (file-attribute-modification-time attributes)
                (file-attribute-link-number attributes)
                (file-attribute-inode-number attributes)))))))

(defun romacs-completion--file-names (candidates table)
  "What `file-name-all-completions' matches for each of TABLE's CANDIDATES.
It matches the directory entry's name, before a directory's slash and,
in `read-file-name-internal', before its dollars are doubled.  Nil when
these are the CANDIDATES themselves."
  (let ((names (mapcar (lambda (c)
                         (let ((name (if (string-suffix-p "/" c) (substring c 0 -1) c)))
                           (if (and (eq table 'read-file-name-internal) (string-search "$" name))
                               (string-replace "$$" "$" name)
                             name)))
                       candidates)))
    (let ((n names) (c candidates))
      (while (and n (eq (car n) (car c))) (setq n (cdr n) c (cdr c)))
      (and n names))))

(defun romacs-completion-candidates (prefix table pred)
  "TABLE's candidates for PREFIX and PRED, or t where TABLE must filter them.
The value is (CANDIDATES . NAMES), NAMES what to match for CANDIDATES
when not themselves.  A collection with no PRED lists them; a file name
table's are kept while their directory is unchanged
\(`romacs-completion--file-candidates')."
  (let ((completion-regexp-list nil))
    (cond
     ((and (null pred) (not (functionp table))) (list (all-completions prefix table)))
     ((when-let* ((key (romacs-completion--file-key prefix table pred)))
        (unless (equal key (car romacs-completion--file-candidates))
          (let ((candidates (all-completions prefix table pred)))
            (setq romacs-completion--file-candidates
                  (cons key (cons candidates (romacs-completion--file-names candidates table))))))
        (cdr romacs-completion--file-candidates)))
     (t t))))

(defun romacs-completion-names (candidates table)
  "What to match for each of TABLE's CANDIDATES, or nil for themselves.
A file name table's entries match by name (`romacs-completion--file-names')."
  (and (memq table '(read-file-name-internal completion-file-name-table))
       (romacs-completion--file-names candidates table)))

(defun romacs-completion-list (prefix table pred)
  "TABLE's candidates for PREFIX and PRED, as (CANDIDATES . NAMES).
As `romacs-completion-candidates', but listed with PRED where TABLE
must filter them, NAMES as `romacs-completion-names'."
  (let ((candidates (romacs-completion-candidates prefix table pred)))
    (if (consp candidates) candidates
      (let ((all (let ((completion-regexp-list nil)) (all-completions prefix table pred))))
        (cons all (romacs-completion-names all table))))))

(provide 'romacs-completion-candidates)
;;; romacs-completion-candidates.el ends here
