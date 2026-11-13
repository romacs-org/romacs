;;; romacs-json.el --- json.el's reader, written for Romacs's engine -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Loaded after every load of GNU's json.el (`romacs-follow-gnu-file'),
;; whose `json-read' this replaces
;; (ledger/divergences/LISP-JSON-READ.md).
;;
;; json.el reads a document a character at a time: two or three
;; primitive calls for every character, which cost far more here than
;; in GNU.  The native parser, `json-parse-buffer', reads the same
;; grammar in one call.  So `json-read' gives a document to it when it
;; can build what json.el's settings ask for, and json.el's own reader
;; reads the rest: any other settings, a reader function redefined or
;; advised, and any document the native parser refuses, so that json.el's
;; errors, and its leniencies, stay as they are.  What the native parser reads is made to be what json.el
;; makes of it: its all-ASCII strings unibyte, the blanks after a number
;; or keyword skipped, and one that is not followed by a delimiter left
;; to json.el to refuse.

;;; Code:

(require 'json)

;; The loadup chain defines it (lisp/romacs/romacs-loadup-bootstrap.el).
(defvar romacs-standard-obarray)

(defvar romacs-json--lisp nil
  "Non-nil while json.el's own reader reads: its nested values too.")

(defconst romacs-json--readers
  '(json-read-string json-read-array json-read-object json-read-keyword
    json-read-number json-read-escaped-char json-new-object json-add-to-object
    json--plist-nreverse json--decode-utf-16-surrogates)
  "The functions json.el's reader calls by name.")

(defconst romacs-json--definitions
  (mapcar (lambda (name) (advice--cd*r (symbol-function name))) romacs-json--readers)
  "Their definitions when json.el defined them, without any advice made
before: a function advised then differs from its definition here.")

(defun romacs-json--own-readers-p ()
  "Whether json.el's reader functions are still json.el's own.
One redefined, advised or bound by `cl-letf' must be called as json.el
calls it, so json.el reads."
  (let ((names romacs-json--readers)
        (definitions romacs-json--definitions))
    (while (and names (eq (symbol-function (car names)) (car definitions)))
      (setq names (cdr names) definitions (cdr definitions)))
    (null names)))

(defconst romacs-json--special-chars
  '((?\" . ?\") (?\\ . ?\\) (?b . ?\b) (?f . ?\f) (?n . ?\n) (?r . ?\r) (?t . ?\t))
  "The escapes the native parser knows: json.el's `json-special-chars' as
json.el defines it, written here so that no binding at load time can
change it.")

(defun romacs-json--native-p ()
  "Whether `json-parse-buffer' builds what json.el's settings ask for,
and what json.el's own reader would build: its reader functions, the
obarray and the escapes as json.el defines them, and symbols for
`json-null' and `json-false', which the normalizing walk never enters."
  (and enable-multibyte-characters
       (romacs-json--own-readers-p)
       ;; A string, vector or list for either is the caller's own, which
       ;; a read returns itself and leaves as it is.
       (symbolp json-null)
       (symbolp json-false)
       ;; The native parser interns object keys in the standard obarray.
       (eq obarray romacs-standard-obarray)
       (equal json-special-chars romacs-json--special-chars)
       (not json-pre-element-read-function)
       (not json-post-element-read-function)
       (memq json-array-type '(vector list))
       (pcase json-object-type
         ('alist (memq json-key-type '(nil symbol)))
         ('plist (memq json-key-type '(nil keyword)))
         ('hash-table (memq json-key-type '(nil string))))))

(defun romacs-json--unibyte (value)
  "VALUE with each string of it that is all ASCII made unibyte, in place.
json.el makes a string by `concat' of its characters, which gives a
unibyte string when they are all ASCII; the native parser's are
multibyte.  Symbols are not entered: `romacs-json--native-p' holds
the caller's `json-null' and `json-false' to symbols."
  (cond ((stringp value)
         (if (= (length value) (string-bytes value)) (string-to-unibyte value) value))
        ((consp value)
         ;; An alist's pairs end in their value.
         (let ((tail value))
           (while (consp tail)
             (setcar tail (romacs-json--unibyte (car tail)))
             (if (consp (cdr tail))
                 (setq tail (cdr tail))
               (setcdr tail (romacs-json--unibyte (cdr tail)))
               (setq tail nil)))
           value))
        ((vectorp value)
         (dotimes (i (length value))
           (aset value i (romacs-json--unibyte (aref value i))))
         value)
        ((hash-table-p value)
         (let ((pairs nil))
           (maphash (lambda (key item) (push (cons key item) pairs)) value)
           (clrhash value)
           (dolist (pair (nreverse pairs) value)
             (puthash (romacs-json--unibyte (car pair)) (romacs-json--unibyte (cdr pair)) value))))
        (t value)))

(defun romacs-json--native ()
  "A list of the value the native parser reads at point, or nil.
Point is after the value and the blanks json.el skips after it; where
the parser refuses, point has not moved."
  (let ((start (point))
        (first (following-char)))
    (or (condition-case nil
            (let ((value (json-parse-buffer
                          :object-type json-object-type
                          :array-type (if (eq json-array-type 'vector) 'array 'list)
                          :null-object json-null :false-object json-false)))
              (cond ((memq first '(?\" ?\[ ?{)) (list (romacs-json--unibyte value)))
                    ;; json.el refuses a number or keyword that runs on.
                    ((looking-at-p "[]\t\n\r ,}]\\|\\'")
                     (skip-chars-forward "\t\n\r ")
                     (list value))))
          (error nil))
        (progn (goto-char start) nil))))

(defun json-read ()
  "Read the JSON value after point and return it, moving point past it.
The value is built as `json-object-type', `json-array-type',
`json-key-type', `json-false' and `json-null' say.  Blanks before the
value are skipped, and after a number or keyword.  At the end of the
buffer this signals `json-end-of-file', and a malformed value signals
`json-error' or one of its kinds."
  (json-skip-whitespace)
  (let ((native (and (not romacs-json--lisp)
                     (romacs-json--native-p)
                     (romacs-json--native))))
    (if native
        (car native)
      ;; json.el's own `json-read'.
      (let ((romacs-json--lisp t))
        (json-readtable-dispatch (char-after))))))

(provide 'romacs-json)
;;; romacs-json.el ends here
