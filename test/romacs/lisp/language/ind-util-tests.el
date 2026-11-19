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

;;; ind-util-tests.el ends here
