;;; ind-util-gnu-record.el --- Record pinned GNU's ISCII coding  -*- lexical-binding: t -*-

;; Run with pinned GNU (`ROMACS_GNU_ORACLE'):
;;   emacs --batch -Q -l ind-util-gnu-record.el -f ind-util-gnu-record > ind-util-gnu.eld
;; It prints one Lisp form: each Devanagari character with an `iscii'
;; property (ucs-devanagari-to-is13194-alist, in code order) as
;; (CHAR BYTES DECODED), BYTES its `in-is13194-devanagari' encoding as a
;; list of bytes and DECODED GNU's decoding of them as a list of code
;; points; then a mixed Latin and Devanagari line as (:mixed TEXT BYTES
;; DECODED).  ind-util-tests.el holds Romacs to it.

;;; Code:

(require 'ind-util)

(defun ind-util-gnu-record ()
  "Print pinned GNU's ISCII coding of the IS 13194 table, as above."
  (let ((entries nil)
        (mixed "Hello नमस्ते world, क़ ऌ ॐ 123."))
    (dotimes (i #x80)
      (let ((char (+ #x0900 i)))
        (when (get-char-code-property char 'iscii)
          (let* ((bytes (encode-coding-string (string char) 'in-is13194-devanagari))
                 (back (decode-coding-string bytes 'in-is13194-devanagari)))
            (push (list char (append bytes nil) (append back nil)) entries)))))
    (let* ((bytes (encode-coding-string mixed 'in-is13194-devanagari))
           (back (decode-coding-string bytes 'in-is13194-devanagari)))
      (let ((print-escape-nonascii t))
        (prin1 (append (nreverse entries)
                       (list (list :mixed (append mixed nil) (append bytes nil) (append back nil))))))
      (terpri))))

;;; ind-util-gnu-record.el ends here
