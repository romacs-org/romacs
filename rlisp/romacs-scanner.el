;;; romacs-scanner.el --- Scanners tabulated from a step function  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A scanner is a state machine over buffer text, written as one pure
;; function and run at the speed of a table:
;;
;;   (define-scanner my-semicolons
;;     (lambda (state char)
;;       (pcase state
;;         ('code (pcase char (?\; '(code . t)) (?\" '(string)) (_ '(code))))
;;         ('string (pcase char (?\" '(code)) (_ '(string))))))
;;     'code
;;     "Semicolons outside strings.")
;;
;;   (romacs-scanner-scan my-semicolons)   ; => (STATE . POSITIONS)
;;   (romacs-scanner-count my-semicolons)  ; => (STATE . COUNT)
;;
;; The step function takes a state and a character and returns
;; (NEW-STATE . MARK); a non-nil MARK marks that character.  It is given
;; each ASCII character as itself and every other character as
;; `romacs-scanner-other', so a scanner cannot tell non-ASCII characters
;; apart.  States are any Lisp values, compared with `equal'; there can be
;; at most 128, and nil can only be the first, since a scan given no state
;; starts in the first.
;;
;; The function must be pure: its result may depend on its two arguments
;; and nothing else.  That is what lets the machine run it ahead of time,
;; on every character in every state it can reach, and keep only the
;; table.  A scan then never calls Lisp.  `romacs-scanner-make' calls the
;; function twice on each input and signals an error if the answers
;; differ, which catches a function that counts or consults a variable
;; that changes; it cannot prove purity.
;;
;; Where the primitive `romacs--scanner-run' is absent the same table is
;; walked in Lisp, with the same results.
;;
;; Each scan hands the primitive the table as a vector, which it packs
;; again: a fixed cost a scan of a large region does not notice and many
;; scans of small regions would.

;;; Code:

(defconst romacs-scanner-other 128
  "What a scanner's step function is given for a character that is not ASCII.")

(defconst romacs-scanner--symbols 129
  "A scanner's alphabet: the ASCII characters and `romacs-scanner-other'.")

(defconst romacs-scanner--states 128
  "The most states a scanner can have: a table entry is a state and a mark.")

(defun romacs-scanner--step (step state char)
  "STEP's answer for STATE and CHAR as (NEW-STATE . MARKED), checked.
STEP is called twice; answers that differ show it is not pure."
  (let ((answer (funcall step state char))
        (again (funcall step state char)))
    (if (not (consp answer))
        (error "Scanner step returned %S, not (STATE . MARK), for %S and %S" answer state char))
    (if (not (and (consp again)
                  (equal (car answer) (car again))
                  (eq (null (cdr answer)) (null (cdr again)))))
        (error "Scanner step is not pure: two answers for %S and %S" state char))
    (cons (car answer) (and (cdr answer) t))))

;;;###autoload
(defun romacs-scanner-make (step initial)
  "Return the scanner whose step function is STEP and first state INITIAL.
STEP is a pure function of a state and a character returning
\(NEW-STATE . MARK); see the commentary of this library.  It is called
here on every character in every state reachable from INITIAL, and never
again."
  (let ((numbers (make-hash-table :test 'equal))
        (states (list initial))
        (pending (list initial))
        (rows nil))
    (puthash initial 0 numbers)
    (while pending
      (let ((state (car pending))
            (row (make-vector romacs-scanner--symbols 0))
            (char 0))
        (setq pending (cdr pending))
        (while (< char romacs-scanner--symbols)
          (let* ((answer (romacs-scanner--step step state char))
                 (number (gethash (car answer) numbers)))
            (if (not number)
                (progn
                  (setq number (hash-table-count numbers))
                  (if (>= number romacs-scanner--states)
                      (error "Scanner has more than %d states" romacs-scanner--states))
                  ;; To a scan, nil is the state it was not given.
                  (if (null (car answer))
                      (error "Scanner state nil can only be the first state"))
                  (puthash (car answer) number numbers)
                  (setq states (nconc states (list (car answer)))
                        pending (nconc pending (list (car answer))))))
            (aset row char (+ number (if (cdr answer) 128 0))))
          (setq char (1+ char)))
        (setq rows (cons row rows))))
    (record 'romacs-scanner (apply #'vconcat (nreverse rows)) (vconcat states) step)))

(defun romacs-scanner-p (object)
  "Return non-nil if OBJECT is a scanner."
  (and (recordp object) (eq (aref object 0) 'romacs-scanner)))

(defun romacs-scanner--check (object)
  "Signal an error unless OBJECT is a scanner."
  (if (not (romacs-scanner-p object))
      (signal 'wrong-type-argument (list 'romacs-scanner-p object))))

(defun romacs-scanner-states (scanner)
  "Return SCANNER's states as a list, its first state first."
  (romacs-scanner--check scanner)
  (append (aref scanner 2) nil))

(defun romacs-scanner-step (scanner)
  "Return the step function SCANNER was tabulated from."
  (romacs-scanner--check scanner)
  (aref scanner 3))

(defun romacs-scanner--number (scanner state)
  "Return the number of STATE in SCANNER.
Nil is the first state; a state SCANNER does not have is an error."
  (let ((states (aref scanner 2)) (number 0) (found (null state)))
    (while (and (not found) (< number (length states)))
      (if (equal (aref states number) state) (setq found t) (setq number (1+ number))))
    (if found number (signal 'wrong-type-argument (list 'romacs-scanner-state-p state)))))

(defun romacs-scanner--walk (table state start end count)
  "Walk TABLE from state number STATE over the text from START to END in Lisp.
The answer of `romacs--scanner-run', for an engine without it."
  (if (or (< start (point-min)) (> end (point-max)) (> start end))
      (signal 'args-out-of-range (list start end)))
  (let ((position start) (marked nil) (total 0))
    (while (< position end)
      (let* ((char (char-after position))
             (entry (aref table (+ (* state romacs-scanner--symbols)
                                   (if (< char 128) char romacs-scanner-other)))))
        (if (>= entry 128)
            (progn (setq total (1+ total))
                   (if (not count) (setq marked (cons position marked)))))
        (setq state (logand entry 127)))
      (setq position (1+ position)))
    (cons state (if count total (nreverse marked)))))

(defun romacs-scanner--run (scanner start end state count)
  "Run SCANNER from START to END starting in STATE; COUNT as the primitive's."
  (romacs-scanner--check scanner)
  (let* ((start (or start (point-min)))
         (end (or end (point-max)))
         (start (if (markerp start) (marker-position start) start))
         (end (if (markerp end) (marker-position end) end))
         (number (romacs-scanner--number scanner state))
         (answer (if (fboundp 'romacs--scanner-run)
                     (romacs--scanner-run (aref scanner 1) number start end count)
                   (romacs-scanner--walk (aref scanner 1) number start end count))))
    (cons (aref (aref scanner 2) (car answer)) (cdr answer))))

;;;###autoload
(defun romacs-scanner-scan (scanner &optional start end state)
  "Run SCANNER over the current buffer's text from START to END.
Return (STATE . POSITIONS): the state after the last character and the
positions, in order, of the characters the step function marked.  START
and END default to the accessible text's ends.  STATE is the state before
START; nil means SCANNER's first state."
  (romacs-scanner--run scanner start end state nil))

;;;###autoload
(defun romacs-scanner-count (scanner &optional start end state)
  "Run SCANNER as `romacs-scanner-scan' does; return (STATE . COUNT).
COUNT is how many characters the step function marked."
  (romacs-scanner--run scanner start end state t))

;;;###autoload
(defmacro define-scanner (name step initial &optional docstring)
  "Define NAME as the scanner with step function STEP and first state INITIAL.
STEP is a pure function of a state and a character that returns
\(NEW-STATE . MARK): the state after the character, and non-nil to mark
it.  It is given every non-ASCII character as `romacs-scanner-other'.
The function is run on every character in every reachable state when
NAME is defined, and never again: a scan reads the resulting table.
DOCSTRING documents NAME.  Run it with `romacs-scanner-scan' or
`romacs-scanner-count'."
  (declare (doc-string 4) (indent defun))
  `(defconst ,name (romacs-scanner-make ,step ,initial) ,docstring))

(provide 'romacs-scanner)
;;; romacs-scanner.el ends here
