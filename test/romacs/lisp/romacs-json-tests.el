;;; romacs-json-tests.el --- romacs-json.el against GNU's own json-read -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; romacs-json.el replaces json.el's `json-read'.  GNU's own is still in
;; lisp/gnu/json.el: these tests read it from that file, run it with
;; json.el's own reader throughout, and hold Romacs's to the same values,
;; points and errors, and to its cost.

;;; Code:

(require 'ert)
(require 'json)
(require 'cl-lib)

(defun romacs-json-tests--gnu ()
  "GNU's own `json-read', read from json.el, its nested values read by
json.el's reader too."
  (let ((definition
         (with-temp-buffer
           (insert-file-contents (locate-file-internal "json.el" load-path))
           (goto-char (point-min))
           (search-forward "\n(defun json-read ()")
           (goto-char (1+ (match-beginning 0)))
           (eval `(function (lambda ,@(cddr (read (current-buffer))))) t))))
    (lambda () (let ((romacs-json--lisp t)) (funcall definition)))))

(defun romacs-json-tests--random (state bound)
  "The next number under BOUND of the sequence STATE, a cons, holds."
  (setcar state (% (+ (* (car state) 1103515245) 12345) 2147483648))
  (% (/ (car state) 65536) bound))

(defun romacs-json-tests--pick (state &rest choices)
  (nth (romacs-json-tests--random state (length choices)) choices))

(defun romacs-json-tests--string (state)
  "A JSON string of escapes, ASCII, multibyte and raw bytes."
  (concat "\""
          (mapconcat (lambda (_)
                       (romacs-json-tests--pick
                        state "a" "bc" "Z" " " "é" "λx" "\\n" "\\\"" "\\\\" "\\/" "\\u00e9"
                        "\\ud83d\\ude00" "\\ud800" "😀" "\\t" "key" "\\b" "\200" "\377" "\x3fff80" "\\x"))
                     (make-list (romacs-json-tests--random state 6) nil) "")
          "\""))

(defun romacs-json-tests--document (state depth)
  "A JSON value of nesting at most DEPTH more levels."
  (pcase (romacs-json-tests--random state (if (> depth 3) 4 7))
    (0 (romacs-json-tests--string state))
    ((or 1 3) (romacs-json-tests--pick
               state "0" "1" "-1" "42" "-0" "3.5" "-2.25" "1e3" "1E-2" "2.5e+3"
               "123456789012345678901234567890" "-0.0" "1e400" "01" "1." "-"))
    (2 (romacs-json-tests--pick state "true" "false" "null" "nul" "truex"))
    (4 (concat "[" (mapconcat (lambda (_) (romacs-json-tests--document state (1+ depth)))
                              (make-list (romacs-json-tests--random state 5) nil)
                              (romacs-json-tests--pick state "," ", " " , "))
               "]"))
    (_ (concat "{"
               (mapconcat (lambda (_)
                            (concat (romacs-json-tests--pick state "\"a\"" "\"b\"" "\"\"" "\"é\"" "\"a b\"")
                                    (romacs-json-tests--pick state ":" ": " " : ")
                                    (romacs-json-tests--document state (1+ depth))))
                          (make-list (romacs-json-tests--random state 5) nil)
                          (romacs-json-tests--pick state "," ", " ",\n  "))
               "}"))))

(defun romacs-json-tests--damaged (state document)
  "DOCUMENT, or a copy of it cut, spliced or run on."
  (let ((at (if (> (length document) 0) (romacs-json-tests--random state (length document)) 0)))
    (pcase (romacs-json-tests--random state 10)
      (0 (substring document 0 at))
      (1 (concat (substring document 0 at)
                 (romacs-json-tests--pick state "x" "\"" "\\" "," "]" "}" ":" "\\u12" "\t" "\0" "\n")
                 (substring document at)))
      (2 (concat document (romacs-json-tests--pick state "x" " x" "," "]" " " "1" " true")))
      (3 (concat (romacs-json-tests--pick state " " "\n\t" "x" "﻿" "\f") document))
      (4 (concat document document))
      (_ document))))

(defun romacs-json-tests--form (value)
  "VALUE as `equal' can compare it: `json-null' and `json-false' themselves
marked, strings with their multibyteness, hash tables as their pairs in
`maphash' order."
  (cond ((and value (eq value json-null)) :the-null)
        ((and value (eq value json-false)) :the-false)
        ((stringp value) (list :string (multibyte-string-p value) value))
        ((consp value) (cons (romacs-json-tests--form (car value))
                             (romacs-json-tests--form (cdr value))))
        ((vectorp value) (vconcat [:vector] (mapcar #'romacs-json-tests--form value)))
        ((hash-table-p value)
         (let ((pairs nil))
           (maphash (lambda (key item)
                      (push (cons (romacs-json-tests--form key) (romacs-json-tests--form item)) pairs))
                    value)
           (list :hash (hash-table-test value) (nreverse pairs))))
        (t value)))

(defun romacs-json-tests--sentinel (state)
  "A value for `json-false' or `json-null': a symbol, or a fresh string,
vector or list a reader must return itself and leave as it is."
  (romacs-json-tests--pick state :json-false nil 'no :null
                           (string-to-multibyte "x") (string-to-multibyte "é")
                           (vector "a" 1) (list "b" 2)))

(defun romacs-json-tests--outcome (reader document narrow unibyte)
  "What READER makes of DOCUMENT after a prefix, and where it leaves point.
NARROW narrows to DOCUMENT; UNIBYTE reads its UTF-8 bytes in a unibyte
buffer."
  (with-temp-buffer
    (when unibyte (set-buffer-multibyte nil))
    (insert "PREFIX " (if unibyte (encode-coding-string document 'utf-8) document))
    (when narrow
      (let ((end (point)))
        (insert " SUFFIX")
        (narrow-to-region 8 end)))
    (goto-char 8)
    (condition-case err
        (list :value (romacs-json-tests--form (funcall reader)) :point (point))
      (error (list :signals err :point (point))))))

(ert-deftest romacs-json-read-is-gnus ()
  "`json-read' reads what GNU's own `json-read' reads.
Over 1,500 generated documents, a third of them damaged, under every
object type, array type and key type, with `json-false' and `json-null'
symbols, strings, vectors or lists of the caller's, a let-bound
`obarray' and a changed `json-special-chars', narrowed or not, in
multibyte and unibyte buffers: the same value (strings' multibyteness
and the caller's `json-false' and `json-null' themselves included), the
same point after it, or the same error at the same point; and the
caller's sentinels unchanged."
  (let ((gnu (romacs-json-tests--gnu))
        (state (list 20261112))
        (signalled 0))
    (dotimes (_ 1500)
      (let* ((document (romacs-json-tests--document state 0))
             (document (if (= 0 (romacs-json-tests--random state 3))
                           (romacs-json-tests--damaged state document)
                         document))
             (json-object-type (romacs-json-tests--pick state 'alist 'alist 'plist 'hash-table))
             (json-array-type (romacs-json-tests--pick state 'vector 'list))
             (json-key-type (romacs-json-tests--pick state nil nil nil 'string 'symbol 'keyword))
             (json-false (romacs-json-tests--sentinel state))
             (json-null (romacs-json-tests--sentinel state))
             (sentinels (copy-tree (list json-false json-null) t))
             (obarray (if (= 0 (romacs-json-tests--random state 4)) (obarray-make) obarray))
             (json-special-chars (if (= 0 (romacs-json-tests--random state 6))
                                     (cons '(?n . ?N) (cons '(?t . ?T) json-special-chars))
                                   json-special-chars))
             (narrow (= 0 (romacs-json-tests--random state 4)))
             (unibyte (= 0 (romacs-json-tests--random state 10)))
             (expected (romacs-json-tests--outcome gnu document narrow unibyte)))
        (when (eq (car expected) :signals) (setq signalled (1+ signalled)))
        (should (equal (romacs-json-tests--outcome #'json-read document narrow unibyte)
                       expected))
        ;; Neither reader changed what the caller gave it.
        (should (equal (list json-false json-null) sentinels))))
    ;; The documents exercise the errors too.
    (should (> signalled 150))))

(ert-deftest romacs-json-read-calls-the-element-hooks ()
  "With `json-pre-element-read-function' or `json-post-element-read-function'
set, json.el's reader reads, so each is called as GNU calls it."
  (let ((gnu (romacs-json-tests--gnu)))
    (dolist (hook '(json-pre-element-read-function json-post-element-read-function))
      (let ((calls nil))
        (let ((json-pre-element-read-function nil)
              (json-post-element-read-function nil))
          (set hook (lambda (&rest arguments) (push arguments calls)))
          (let ((ours (romacs-json-tests--outcome #'json-read "{\"a\": [1, {\"b\": 2}]}" nil nil))
                (ours-calls (prog1 calls (setq calls nil)))
                (expected (romacs-json-tests--outcome gnu "{\"a\": [1, {\"b\": 2}]}" nil nil)))
            (should (equal ours expected))
            (should (equal ours-calls calls))
            (should calls)))))))

(ert-deftest romacs-json-read-calls-redefined-readers ()
  "A json.el reader function redefined, advised or bound by `cl-letf' is
called as json.el calls it, and the native parser reads again once it is
json.el's own."
  (dolist (case '((json-read-string . "\"a\"") (json-read-array . "[1]")
                  (json-read-object . "{\"a\":1}") (json-read-keyword . "true")
                  (json-read-number . "7") (json-add-to-object . "{\"a\":1}")
                  (json-read-escaped-char . "\"\\n\"")))
    (let ((called nil))
      (cl-letf (((symbol-function (car case))
                 (let ((own (symbol-function (car case))))
                   (lambda (&rest arguments) (setq called t) (apply own arguments)))))
        (with-temp-buffer
          (insert (cdr case))
          (goto-char (point-min))
          (json-read)))
      (should called)))
  (let* ((called nil)
         (advice (lambda (&rest _) (setq called t))))
    (advice-add 'json-read-number :before advice)
    (unwind-protect
        (should (equal (json-read-from-string "[1, 2]") [1 2]))
      (advice-remove 'json-read-number advice))
    (should called)
    (setq called nil)
    (should (romacs-json--own-readers-p))
    (should (equal (json-read-from-string "[1, 2]") [1 2]))
    (should-not called)))

(ert-deftest romacs-json-read-keeps-its-recorded-differences ()
  "The differences DIVERGENCES LISP-JSON-READ records, as they are.
A value the native parser reads leaves the match data as they were,
where json.el's reader leaves its own; a key interned before the native
parser refuses the document stays interned, where json.el's reader
interns a key only once its value is read; and nesting json.el's
reader cannot reach under `max-lisp-eval-depth' is read."
  (let ((gnu (romacs-json-tests--gnu)))
    (string-match "\\(q\\)" "pq")
    (should (equal (json-read-from-string "[1, true]") [1 t]))
    (should (equal (match-data) '(1 2 1 2)))
    (let ((name (format "romacs-json-tests-key-%d" (random most-positive-fixnum))))
      (should-not (intern-soft name))
      (should (eq (car (romacs-json-tests--outcome #'json-read (format "{\"%s\": ]" name) nil nil))
                  :signals))
      (should (intern-soft name)))
    (let ((deep (concat (make-string 1000 ?\[) (make-string 1000 ?\])))
          (max-lisp-eval-depth 1600))
      ;; Not through `romacs-json-tests--form', whose walk would itself
      ;; reach the bound.
      (dolist (case (list (cons gnu nil) (cons #'json-read t)))
        (should (eq (condition-case nil
                        (with-temp-buffer
                          (insert deep)
                          (goto-char (point-min))
                          (funcall (car case))
                          t)
                      (excessive-lisp-nesting nil))
                    (cdr case)))))))

(ert-deftest romacs-json-read-refuses-deep-nesting-as-gnus ()
  "100,000 nested arrays or objects signal what GNU's own reader signals,
and the session goes on.  The native parser refuses past 10,000 levels
and the walk after it is bounded by `max-lisp-eval-depth', so neither
reaches the machine's own stack, and json.el's reader signals."
  (let ((gnu (romacs-json-tests--gnu)))
    (dolist (document (list (concat (make-string 100000 ?\[) "0" (make-string 100000 ?\]))
                            (concat (apply #'concat (make-list 100000 "{\"a\":")) "0"
                                    (make-string 100000 ?}))))
      ;; The same error; where it stops depends on the frames each level
      ;; takes, which differ by a few levels.
      (let ((ours (romacs-json-tests--outcome #'json-read document nil nil)))
        (should (eq (car ours) :signals))
        (should (eq (car (nth 1 ours))
                    (car (nth 1 (romacs-json-tests--outcome gnu document nil nil))))))))
  (should (equal (json-read-from-string "[[1]]") [[1]])))

(defconst romacs-json-tests--objects
  (let ((items nil))
    (dotimes (i 200)
      (push (format "{\"id\": %d, \"name\": \"item-%d\", \"tags\": [\"a\", \"b\", \"c%d\"], \"score\": %d.5, \"ok\": true, \"nested\": {\"x\": [1, 2, 3, %d], \"y\": null}}"
                    i i (% i 7) i i)
            items))
    (concat "[" (mapconcat #'identity items ", ") "]"))
  "An array of 200 objects, 23,000 characters.")

(defun romacs-json-tests--instructions (reader)
  "The fewest instructions of three reads of the 200 objects by READER."
  (let ((least nil))
    (dotimes (_ 3)
      (with-temp-buffer
        (insert romacs-json-tests--objects)
        (goto-char (point-min))
        (let ((before (romacs-instructions)))
          (funcall reader)
          (let ((spent (- (romacs-instructions) before)))
            (setq least (if least (min least spent) spent))))))
    least))

(ert-deftest romacs-json-read-cost ()
  "A document costs the native parser's read and one walk of its value.
200 objects: under 1,500 instructions a character of the document.
GNU's own reader, run beside it, makes primitive calls for every
character: over five times as many.  If that stops being so, json.el's
reader has become cheap here, and romacs-json.el is to delete."
  (should (romacs-instructions))
  (let ((ours (romacs-json-tests--instructions #'json-read))
        (gnus (romacs-json-tests--instructions (romacs-json-tests--gnu)))
        (size (length romacs-json-tests--objects)))
    (message "romacs-json: %d instructions a character, GNU's json-read %d"
             (/ ours size) (/ gnus size))
    (should (< ours (* 1500 size)))
    (should (< (* 5 ours) gnus))))

(ert-deftest romacs-json-read-survives-a-reload-of-json ()
  "A load of json.el again restores GNU's `json-read', and romacs-json.el
follows it: the definition stays Romacs's, and its snapshot of json.el's
reader functions is of the new ones, so the native parser reads again."
  (load "json" nil t)
  (should (equal (file-name-nondirectory (symbol-file 'json-read 'defun)) "romacs-json.el"))
  (should (romacs-json--own-readers-p))
  (should (equal (json-read-from-string "[1]") [1])))

(ert-deftest romacs-json-read-survives-json-loaded-by-another-name ()
  "json.el loaded through a symlink of another name restores GNU's
`json-read' too, and romacs-json.el follows the `json' feature it
provides, whatever the file is called."
  (let ((directory (make-temp-file "romacs-json-tests" t)))
    (unwind-protect
        (let ((alias (expand-file-name "alias-json.el" directory)))
          (make-symbolic-link (locate-file-internal "json.el" load-path) alias)
          (load alias nil t t)
          (should (equal (file-name-nondirectory (symbol-file 'json-read 'defun)) "romacs-json.el"))
          (should (equal (json-read-from-string "[1]") [1])))
      (delete-directory directory t))))

(ert-deftest romacs-json-read-calls-advice-made-before-a-reload ()
  "A reader function advised before json.el loads again is called as
json.el calls it: the reload's snapshot of json.el's reader functions
holds their own definitions, never the advice around them."
  (let ((advice (lambda (function &rest arguments) (* 10 (apply function arguments)))))
    (advice-add 'json-read-number :around advice)
    (unwind-protect
        (progn
          (load "json" nil t)
          (should (equal (json-read-from-string "[1]") [10])))
      (advice-remove 'json-read-number advice))
    (should (romacs-json--own-readers-p))
    (should (equal (json-read-from-string "[1]") [1]))))

(ert-deftest romacs-json-read-knows-json-el-s-escapes-whatever-its-load-saw ()
  "romacs-json.el loaded while `json-special-chars' is let-bound still
takes json.el's own escapes for the native parser's: under a changed
binding later, json.el's reader reads, as GNU's does."
  (let ((gnu (romacs-json-tests--gnu))
        (changed (cons '(?n . ?N) json-special-chars)))
    (let ((json-special-chars changed))
      (load "romacs-json" nil t))
    (let ((json-special-chars changed))
      (should (equal (romacs-json-tests--outcome #'json-read "\"a\\nb\"" nil nil)
                     (romacs-json-tests--outcome gnu "\"a\\nb\"" nil nil))))
    (should (equal (json-read-from-string "\"a\\nb\"") "a\nb"))))

;;; romacs-json-tests.el ends here
