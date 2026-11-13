;;; romacs-mule-tests.el --- romacs-mule.el against GNU's own definitions -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; romacs-mule.el replaces definitions of GNU's mule.el.  GNU's own are
;; still in lisp/gnu/: these tests read them from that file and hold
;; Romacs's to the same tables and the same errors, and to their cost.

;;; Code:

(require 'ert)

(defun romacs-mule-tests--gnu (name)
  "GNU's own definition of the function NAME, read from mule.el."
  (with-temp-buffer
    (insert-file-contents (locate-file-internal "international/mule.el" load-path))
    (goto-char (point-min))
    (search-forward (format "\n(defun %s " name))
    (goto-char (1+ (match-beginning 0)))
    (eval `(function (lambda ,@(cddr (read (current-buffer))))) t)))

(defun romacs-mule-tests--content (table)
  "All a package can read of the translation table TABLE.
`equal' does not compare char tables' entries, so this lists them: the
subtype, the parent, each entry `map-char-table' gives, the lookup
length, and the same of the reverse table."
  (let ((entries nil)
        (reverse (char-table-extra-slot table 0)))
    (map-char-table
     (lambda (key value)
       ;; The walk may reuse a range's cons.
       (push (cons (if (consp key) (cons (car key) (cdr key)) key) value) entries))
     table)
    (list (char-table-subtype table) (char-table-parent table) (nreverse entries)
          (char-table-extra-slot table 1)
          (if (char-table-p reverse) (romacs-mule-tests--content reverse) reverse))))

(defun romacs-mule-tests--outcome (builder alist)
  "What BUILDER makes of ALIST: its table's content, or the error it signals."
  (condition-case err
      (romacs-mule-tests--content (funcall builder alist))
    (error (list :signals err))))

(defun romacs-mule-tests--random (state bound)
  "The next number under BOUND of the sequence STATE, a cons, holds.
The same sequence on every run and engine."
  (setcar state (% (+ (* (car state) 1103515245) 12345) 2147483648))
  (% (/ (car state) 65536) bound))

(defconst romacs-mule-tests--characters
  [?a ?b ?c #x3b1 #x1100 #x1161 #xac00 #xac01 #xffff #x10ffff #x110000 #x3fffff 0]
  "Few characters, so that mappings share them; each plane's edges.")

(defconst romacs-mule-tests--first-characters [?a ?b ?c]
  "Characters drawn half the time, so that mappings collide on them.")

(defun romacs-mule-tests--side (state lists)
  "One side of a mapping: a character, a sequence of them, nothing, or,
where LISTS is non-nil, one of those lists, which mappings share and
builders append to."
  (let ((char (lambda ()
                (let ((from (if (= 0 (romacs-mule-tests--random state 2))
                                romacs-mule-tests--first-characters
                              romacs-mule-tests--characters)))
                  (aref from (romacs-mule-tests--random state (length from)))))))
    (pcase (romacs-mule-tests--random state 15)
      ((or 0 1 2 3 4) (funcall char))
      ((or 5 6) (vector (funcall char)))
      ((or 7 8) (vector (funcall char) (funcall char)))
      (9 (vector (funcall char) (funcall char) (funcall char)))
      (10 (if (= 0 (romacs-mule-tests--random state 2)) nil []))
      ((or 11 12 13)
       (if lists
           (aref lists (romacs-mule-tests--random state (length lists)))
         (vector (funcall char) (funcall char))))
      (_ (string (funcall char) ?z)))))

(defun romacs-mule-tests--alist (seed)
  "An alist of up to 40 mappings drawn from SEED, and the lists it shares.
Each call builds them afresh: a builder may append to the lists.  A
list is only ever what a mapping maps to, and only for one seed in
five: the builder appends to it and then signals, as it cannot be
mapped back, so the other four seeds in five build both tables whole."
  (let* ((state (list seed))
         (lists (vector (list (cons [?x] ?y)) (list (cons [?a ?q] ?r) (cons [?b] ?s))))
         (shared (and (= 0 (romacs-mule-tests--random state 5)) lists))
         (alist nil))
    (dotimes (_ (romacs-mule-tests--random state 41))
      (push (cons (romacs-mule-tests--side state nil) (romacs-mule-tests--side state shared)) alist))
    (cons alist lists)))

(defun romacs-mule-tests--fresh-outcome (builder seed)
  "What BUILDER makes of SEED's alist, and its shared lists afterwards."
  (let ((made (romacs-mule-tests--alist seed)))
    (list (romacs-mule-tests--outcome builder (car made)) (cdr made))))

(defun romacs-mule-tests--hangul ()
  "The 11,172 Hangul syllables to their jamo, as ucs-normalize.el lists them."
  (let ((alist nil))
    (dotimes (i 11172)
      (push (cons (+ #xac00 i)
                  (if (= 0 (% i 28))
                      (vector (+ #x1100 (/ i 588)) (+ #x1161 (/ (% i 588) 28)))
                    (vector (+ #x1100 (/ i 588)) (+ #x1161 (/ (% i 588) 28)) (+ #x11a7 (% i 28)))))
            alist))
    alist))

(ert-deftest romacs-mule-translation-tables-are-gnus ()
  "`make-translation-table-from-alist' builds GNU's tables.
Over 1,500 generated alists (characters and sequences on either side,
empty sides, strings, characters mapped twice and sequences sharing a
first character; in one alist of five, lists two mappings share, which
end in GNU's signal) and ucs-normalize.el's Hangul, the outcome is what
GNU's own definition gives: every entry of both tables, the lookup
lengths, the reverse table, the signal, and what was appended to the
lists given.  Each builder gets its own copy of an alist."
  (let ((gnu (romacs-mule-tests--gnu 'make-translation-table-from-alist)))
    (dotimes (seed 1500)
      (should (equal (romacs-mule-tests--fresh-outcome #'make-translation-table-from-alist seed)
                     (romacs-mule-tests--fresh-outcome gnu seed))))
    (let ((hangul (romacs-mule-tests--hangul)))
      (should (equal (romacs-mule-tests--outcome #'make-translation-table-from-alist hangul)
                     (romacs-mule-tests--outcome gnu hangul))))))

(ert-deftest romacs-mule-translation-tables-signal-as-gnus ()
  "What GNU's definition signals for an alist, Romacs's signals too.
A side that is no character and no sequence, a sequence whose first
element is no character, a list for a side (appended to in place before
the reverse table refuses it), and an ALIST that is no list of conses."
  (let ((gnu (romacs-mule-tests--gnu 'make-translation-table-from-alist)))
    (dolist (alist '(((-1 . ?a)) ((#x400000 . ?a)) ((foo . ?a)) ((1.5 . ?a))
                     ((?a . foo)) (([foo] . ?a)) (([-1 ?a] . ?b))
                     ((?a . ?b) ([?a ?c] . [1.5 ?d])) (?a) foo ((?a . ?b) . foo)
                     ((?a . ?b) (?b . ?a) ([?a ?b] . ?c) (?a . bar))))
      (let ((ours (romacs-mule-tests--outcome #'make-translation-table-from-alist alist)))
        (should (eq (car ours) :signals))
        (should (equal ours (romacs-mule-tests--outcome gnu alist)))))
    ;; A list for a TO is stored as given and appended to, in GNU as here.
    (dolist (builder (list gnu #'make-translation-table-from-alist))
      (let ((given (list (cons [?x] ?y))))
        (should (eq (car (romacs-mule-tests--outcome
                          builder (list (cons ?a given) (cons [?a ?b] ?c))))
                    :signals))
        (should (equal given '(([?x] . ?y) ([?a ?b] . ?c))))))))

(ert-deftest romacs-mule-translation-tables-append-to-a-shared-list-as-gnus ()
  "A list two characters map to, which a third mapping of each appends
to, ends as GNU's definition leaves it: every appended mapping kept, in
GNU's order, and the same table or error.  Built afresh for each builder."
  (let ((made (lambda ()
                (let ((s (list (cons [?x] ?y))))
                  (cons (list (cons ?a s) (cons ?b s) (cons [?a ?c] ?d) (cons [?b ?e] ?f)
                              (cons [?a ?g] ?h))
                        s)))))
    (let ((ours (funcall made))
          (gnus (funcall made)))
      (should (equal (list (romacs-mule-tests--outcome #'make-translation-table-from-alist (car ours))
                           (cdr ours))
                     (list (romacs-mule-tests--outcome
                            (romacs-mule-tests--gnu 'make-translation-table-from-alist) (car gnus))
                           (cdr gnus))))
      (should (member '([?b ?e] . ?f) (cdr ours))))))

(defun romacs-mule-tests--instructions (builder alist)
  "The fewest instructions of three calls of BUILDER on ALIST.
A collection may fall in one of them."
  (let ((least nil))
    (dotimes (_ 3)
      (let ((before (romacs-instructions)))
        (funcall builder alist)
        (let ((spent (- (romacs-instructions) before)))
          (setq least (if least (min least spent) spent)))))
    least))

(ert-deftest romacs-mule-translation-table-cost ()
  "A mapping costs one write of the char table in each direction.
ucs-normalize.el's Hangul, 11,172 mappings whose reverse entries are 19
lists of 588: under 4,800 instructions a mapping for both tables.  GNU's
own definition, run beside it, reads and writes the table for every
mapping and walks each list to append to it: over three times as many.
If that stops being so, GNU's definition has become cheap here, and
romacs-mule.el's is to delete."
  (should (romacs-instructions))
  (let* ((hangul (romacs-mule-tests--hangul))
         (ours (romacs-mule-tests--instructions #'make-translation-table-from-alist hangul))
         (gnus (romacs-mule-tests--instructions
                (romacs-mule-tests--gnu 'make-translation-table-from-alist) hangul)))
    (message "romacs-mule: %d instructions a mapping, GNU's definition %d"
             (/ ours 11172) (/ gnus 11172))
    (should (< ours (* 4800 11172)))
    (should (< (* 3 ours) gnus))))

(ert-deftest romacs-mule-translation-tables-survive-a-reload-of-mule ()
  "A load of mule.el again restores GNU's definition, and romacs-mule.el
follows it, so the definition stays Romacs's."
  (load "international/mule.el" nil t t)
  (should (equal (file-name-nondirectory (symbol-file 'make-translation-table-from-alist 'defun))
                 "romacs-mule.el"))
  (should (char-table-p (make-translation-table-from-alist '((?a . ?b))))))

;;; romacs-mule-tests.el ends here
