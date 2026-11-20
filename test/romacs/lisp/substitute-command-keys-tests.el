;;; substitute-command-keys-tests.el --- A string with nothing to substitute, as GNU returns it  -*- lexical-binding: t -*-

;; `substitute-command-keys' makes a string with no `\\', grave accent or
;; apostrophe without its temporary buffer.  The result is the one GNU's
;; loop gives: a fresh multibyte copy with the argument's properties.
;; Every expected value is pinned GNU 45b0f769's.

(require 'ert)

(defun substitute-command-keys-tests--shape (string)
  "What callers can see of `substitute-command-keys' of STRING."
  (let ((result (substitute-command-keys string)))
    (list result (eq result string) (multibyte-string-p result)
          (equal-including-properties result string))))

(ert-deftest substitute-command-keys-plain-is-a-fresh-copy ()
  (should (equal (substitute-command-keys-tests--shape "plain text") '("plain text" nil t t)))
  (should (equal (substitute-command-keys-tests--shape "") '("" nil t t)))
  (should (equal (substitute-command-keys-tests--shape "λ multibyte") '("λ multibyte" nil t t)))
  (should-not (substitute-command-keys nil)))

(ert-deftest substitute-command-keys-plain-keeps-properties ()
  (let ((faced (propertize "λ faced" 'face 'italic 'x 1))
        (part (concat "x" (propertize "y" 'help-echo "h"))))
    (should (equal-including-properties (substitute-command-keys faced) faced))
    (should (equal-including-properties (substitute-command-keys part) part))))

(ert-deftest substitute-command-keys-plain-unibyte-becomes-multibyte ()
  (let ((ascii (string-to-unibyte "unibyte ascii"))
        (raw (string-to-unibyte "raw \377 byte"))
        (faced (let ((u (string-to-unibyte "uni prop"))) (put-text-property 0 3 'face 'bold u) u)))
    (should (equal (substitute-command-keys-tests--shape ascii) '("unibyte ascii" nil t t)))
    ;; A raw byte becomes its eight-bit character, as an insert makes it.
    (should (equal (substitute-command-keys raw) (string-to-multibyte raw)))
    (should (multibyte-string-p (substitute-command-keys raw)))
    ;; One with properties keeps them (the full loop makes it).
    (should (equal-including-properties (substitute-command-keys faced) faced))
    (should (multibyte-string-p (substitute-command-keys faced)))))

(ert-deftest substitute-command-keys-still-substitutes ()
  (let ((text-quoting-style 'curve))
    (should (equal (substitute-command-keys "has `quote'") "has ‘quote’"))
    (should (equal (substitute-command-keys "it's") "it’s")))
  (should (equal (substitute-command-keys "back\\slash") "back\\slash")))

;;; substitute-command-keys-tests.el ends here
