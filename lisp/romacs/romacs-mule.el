;;; romacs-mule.el --- mule.el's table builders, written for Romacs's engine -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Loaded after GNU's international/mule.el, and after every later load
;; of it (`romacs-follow-gnu-file'), whose definitions these replace (ledger/divergences/LISP-TRANSLATION-TABLE.md).
;;
;; GNU's `make-translation-table-from-alist' reads the char table and
;; writes it back for every mapping, in both directions, and appends to
;; an entry's list with `nconc', which walks the list each time: 588
;; mappings behind each leading consonant of ucs-normalize.el's Hangul.
;; A char table access is a primitive call here, while a read or write
;; of a Lisp vector is none.  So this one keeps what it must know of each
;; character in Lisp vectors, writes the char table when a character
;; first appears (and once more when its value becomes a list), and
;; appends from where the list last ended.  The
;; tables are GNU's, entry for entry.

;;; Code:

(defun romacs-mule--translation-map (alist reverse)
  "A translation table of ALIST's FROM to TO mappings, TO to FROM if REVERSE.
One direction of `make-translation-table-from-alist', with GNU's
entries: a character mapped once, as a character, holds what it becomes;
any other holds a list of (FROM-VECTOR . TO) in ALIST's order.

PAGES says what a character's entry is so far, without a read of the
table: nil for none, t for a value stored as given, else the cons its
list last ended at, from which the next mapping is appended."
  (let ((table (make-char-table 'translation-table))
        (pages (make-vector (/ (1+ (max-char)) 256) nil))
        (max-lookup 1))
    (dolist (mapping alist)
      (let* ((from (if reverse (cdr mapping) (car mapping)))
             (to (if reverse (car mapping) (cdr mapping)))
             ;; `integerp' first: it is no call, and a reverse table's
             ;; FROMs are mostly vectors.
             (char (and (integerp from) (characterp from)))
             (size (if char 0 (length from)))
             (key (cond (char from) ((> size 0) (aref from 0)))))
        (setq max-lookup (max max-lookup size))
        (when key
          ;; GNU reads the table at KEY, which signals for one that is
          ;; no character.
          (or char (characterp key) (aref table key))
          (let* ((page (or (aref pages (/ key 256))
                           (aset pages (/ key 256) (make-vector 256 nil))))
                 (slot (% key 256))
                 (tail (aref page slot)))
            (cond
             ((and (null tail) char)
              ;; A nil TO is no entry: the next mapping of KEY is its first.
              (when to
                (aset table key to)
                (aset page slot t)))
             ((null tail)
              (aset page slot (aset table key (list (cons from to)))))
             (t
              (let ((new (list (cons (if char (vector from) from) to))))
                (if (consp tail)
                    ;; The last cons of the list, unless the list is
                    ;; another character's too, which may have appended
                    ;; to it since: `nconc' finds its end.
                    (nconc tail new)
                  ;; A value stored as given becomes a list's first
                  ;; mapping; one that is a list already is appended to,
                  ;; as GNU's `nconc' does.
                  (let ((value (aref table key)))
                    (if (consp value)
                        (nconc value new)
                      (aset table key (cons (cons (vector key) value) new)))))
                (aset page slot new))))))))
    (set-char-table-extra-slot table 1 max-lookup)
    table))

(defun make-translation-table-from-alist (alist)
  "Return a translation table for the mappings of ALIST.
Each element of ALIST is (FROM . TO), FROM and TO each a character or a
vector of characters: FROM, one character or a sequence of them,
translates to TO.  The table's first extra slot holds the table of the
reverse mappings, TO to FROM.

A FROM or TO that is nil or an empty vector stands for nothing.  With
such a TO, FROM translates to nothing and the reverse table has no
mapping for the element; with such a FROM, the element is in the reverse
table alone."
  (let ((table (romacs-mule--translation-map alist nil)))
    (set-char-table-extra-slot table 0 (romacs-mule--translation-map alist t))
    table))

(provide 'romacs-mule)
;;; romacs-mule.el ends here
