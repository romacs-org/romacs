;;; ind-util-tests.el --- Indian conversions on Unicode text -*- lexical-binding: t; -*-

;; GNU's map from old Emacs Devanagari glyph characters to UCS keyed
;; characters above #x10FFFF, which no buffer here holds; it is empty
;; (ledger/divergences/VM2-INDIAN-2-COLUMN.md), and its converter, like
;; `indian-compose-region', runs on Unicode text and changes nothing.

(require 'ert)
(require 'ind-util)

(defconst ind-util-tests--text "क्षत्रिय हिन्दी"
  "Devanagari, every character of which has a `composition-function-table' entry.")

(ert-deftest ind-util-2-column-conversion-leaves-unicode-alone ()
  (should-not indian-2-colum-to-ucs)
  (with-temp-buffer
    (insert ind-util-tests--text)
    (indian-2-column-to-ucs-region (point-min) (point-max))
    (should (equal (buffer-string) ind-util-tests--text))))

(ert-deftest ind-util-compose-skips-rule-entries ()
  ;; A `composition-function-table' entry is a list of rule vectors, not
  ;; a function: GNU's `fboundp' on it signalled, for any Devanagari.
  (should (consp (aref composition-function-table ?क)))
  (should (equal (indian-compose-string ind-util-tests--text) ind-util-tests--text)))

;; IS 13194 (ISCII): Romacs's coding goes through raw bytes
;; (ledger/divergences/VM2-ISCII-RAW-BYTES.md).  ind-util-gnu.eld is
;; pinned GNU's recording (ind-util-gnu-record.el beside it): each
;; character of the table with its `in-is13194-devanagari' bytes and
;; GNU's decoding of them, and a mixed Latin and Devanagari line.

(defconst ind-util-tests--gnu
  (with-temp-buffer
    (insert-file-contents
     (expand-file-name "ind-util-gnu.eld"
                       (file-name-directory (or load-file-name buffer-file-name))))
    (read (current-buffer)))
  "Pinned GNU's ISCII coding.
Its (CHAR BYTES DECODED) entries, then (:mixed TEXT BYTES DECODED).")

(defconst ind-util-tests--to-original
  '(#x090c #x093d #x0944 #x0950 #x0960 #x0961 #x0962 #x0963 #x0965 #x0951 #x0952 #x0911)
  "Characters whose ISCII bytes GNU decodes to another text, and Romacs to them.
Vocalic L, avagraha, vowel sign vocalic RR, OM, vocalic RR and LL,
vowel signs vocalic L and LL (a letter or sign followed by nukta),
double danda (two dandas), the stress signs udatta and anudatta
(GNU leaves their ATR byte as a charset character above #x10FFFF), and
candra O (#xb2, which GNU decodes as candra E, whose byte is #xae).
The Unicode Standard 15.0, section 12.1, has ISCII-1991 text convert
to Unicode and back without loss; the pairs are GNU's own encoding
table's, none checked against IS 13194:1991 itself.")

(defun ind-util-tests--encode (codes)
  "The ISCII bytes of the text of CODES, as a list."
  (with-temp-buffer
    (insert (apply #'string codes))
    (indian-ucs-to-iscii-region (point-min) (point-max))
    (append (encode-coding-string (buffer-string) 'raw-text) nil)))

(defun ind-util-tests--decode (bytes)
  "The text the ISCII BYTES decode to, as a list of code points."
  (with-temp-buffer
    (insert (string-to-multibyte (apply #'unibyte-string bytes)))
    (indian-iscii-to-ucs-region (point-min) (point-max))
    (append (buffer-string) nil)))

(ert-deftest ind-util-iscii-encodes-every-entry-as-gnu-does ()
  (let ((entries (butlast ind-util-tests--gnu)))
    (should (= (length entries) 128))
    (dolist (entry entries)
      (should (equal (list (car entry) (ind-util-tests--encode (list (car entry))))
                     (list (car entry) (nth 1 entry)))))))

(ert-deftest ind-util-iscii-decodes-every-entry ()
  ;; As GNU does, but for the twelve GNU decodes to another text.
  (let ((differing nil))
    (dolist (entry (butlast ind-util-tests--gnu))
      (pcase-let ((`(,char ,bytes ,gnu) entry))
        (if (memq char ind-util-tests--to-original)
            (progn
              (push char differing)
              (should-not (equal gnu (list char)))
              (should (equal (list char gnu (ind-util-tests--decode bytes)) (list char gnu (list char)))))
          (should (equal (list char (ind-util-tests--decode bytes)) (list char gnu))))))
    (should (equal (sort differing #'<) (sort (copy-sequence ind-util-tests--to-original) #'<)))))

(ert-deftest ind-util-iscii-round-trips-a-mixed-line ()
  ;; Latin, Devanagari, a consonant with nukta (decomposed, as NFC has
  ;; it), vocalic L and OM: GNU's bytes, and back to the line itself.
  (pcase-let ((`(:mixed ,text ,bytes ,_gnu) (car (last ind-util-tests--gnu))))
    (should (equal (ind-util-tests--encode text) bytes))
    (should (equal (ind-util-tests--decode bytes) text))))

(ert-deftest ind-util-iscii-coding-is-raw-bytes ()
  (should (eq (coding-system-type 'in-is13194-devanagari) 'raw-text))
  (should (eq (coding-system-get 'in-is13194-devanagari :post-read-conversion)
              'in-is13194-post-read-conversion)))

;;; ind-util-tests.el ends here
