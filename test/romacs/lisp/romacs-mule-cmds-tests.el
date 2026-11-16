;;; romacs-mule-cmds-tests.el --- A character from its name, without the table -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)

(defconst romacs-mule-cmds-tests--names
  '("COLON" "SMALL COLON" "FULLWIDTH COLON" "PRESENTATION FORM FOR VERTICAL COLON"
    "KHMER SIGN CAMNUC PII KUUH" "HYPHEN-MINUS" "LATIN SMALL LETTER A WITH GRAVE" "GREEK SMALL LETTER LAMDA"
    ;; Kept whole, after computed names in their blocks.
    "HEXAGRAM FOR THE CREATIVE HEAVEN" "HANGUL JONGSEONG SSANGMIEUM"
    ;; Not a character's name: an old name, a spelling `ucs-names' adds,
    ;; a computed name, its own special case, a parsed one, none.
    "LINE FEED (LF)" "GREEK SMALL LETTER LAMBDA" "HANGUL SYLLABLE GA" "BELL (BEL)"
    "VARIATION SELECTOR-17" "NO SUCH CHARACTER")
  "Names of each kind `char-from-name' is given.")

(ert-deftest romacs-mule-cmds-a-name-is-found-without-the-table ()
  ;; ?\N{...} in a library (TRAMP's) made the first `char-from-name'
  ;; build `ucs-names', every name, a third of a second.  A character's
  ;; own name is now found in the blocks that hold its words.
  (let ((ucs-names nil) (ucs-names--direct 3))
    (should (eq (char-from-name "SMALL COLON") #xFE55))
    (should (eq (char-from-name "hyphen-minus" t) ?-))
    (should (eq (read "?\\N{KHMER SIGN CAMNUC PII KUUH}") #x17D6))
    (should-not ucs-names)
    ;; A caller that asks for more gets the table.
    (should (eq (char-from-name "COLON") ?:))
    (should ucs-names)))

(ert-deftest romacs-mule-cmds-a-name-is-the-tables ()
  ;; Found directly or not (the table is made for the rest), the answer
  ;; is the one `ucs-names' gives.
  (let* ((table (ucs-names))
         (tabled (mapcar #'char-from-name romacs-mule-cmds-tests--names))
         (direct (let ((ucs-names nil) (ucs-names--direct most-positive-fixnum))
                   (cl-letf (((symbol-function 'ucs-names) (lambda () table)))
                     (mapcar #'char-from-name romacs-mule-cmds-tests--names)))))
    (should (equal direct tabled))
    (should (equal (cl-subseq direct 0 10) '(?: #xFE55 #xFF1A #xFE13 #x17D6 ?- #xE0 #x3BB #x4DC0 #xD7E0)))
    (should-not (car (last direct)))))

(ert-deftest romacs-mule-cmds-every-name-is-the-tables ()
  :tags '(:expensive-test)
  ;; Every 25th key of `ucs-names' (all of them take a quarter of an
  ;; hour): a character's own name kept in the table is found directly,
  ;; as that character; a computed name and any other key are not.
  (let ((names (unicode-property-table-internal 'name)) (i 0) (wrong nil))
    (maphash (lambda (name char)
               (when (zerop (% (setq i (1+ i)) 25))
                 (let ((direct (ucs-names--current name))
                       (kept (and (equal (get-char-code-property char 'name) name)
                                  (stringp (aref names char))
                                  (cl-some (lambda (r) (<= (car r) char (cdr r))) ucs-names--ranges))))
                   (unless (if kept (eq direct char) (null direct)) (push (list name char direct) wrong)))))
             (ucs-names))
    (should-not wrong)))

;;; romacs-mule-cmds-tests.el ends here
