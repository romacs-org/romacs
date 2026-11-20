;;; tibetan-tests.el --- Tibetan text stays Unicode -*- coding: utf-8; lexical-binding: t; -*-

;; Romacs keeps Tibetan text in Unicode (ledger/divergences/VM2-TIBETAN-UNICODE.md):
;; GNU's precomposed glyph codes and its invisible vowel sign, above
;; #x10FFFF, which no buffer here holds, are their Unicode stacks and
;; nothing.  The expected strings are pinned GNU's output for the same
;; keys after `tibetan-canonicalize-for-unicode-region'.

(require 'ert)
(require 'quail)
(require 'tibet-util)

(declare-function quail-tibetan-update-translation "quail/tibetan" (control-flag))

(defun tibetan-tests--typed (key)
  "The text the Wylie input method inserts for KEY, as
`quail-tibetan-update-translation' makes it."
  (with-temp-buffer
    (insert (tibetan-compose-string (tibetan-transcription-to-tibetan key)))
    (buffer-substring-no-properties (point-min) (point-max))))

(ert-deftest tibetan-wylie-syllables-insert-unicode ()
  ;; "ka" and "bkra" carry the inherent vowel, which GNU wrote as an
  ;; invisible sign no Unicode buffer holds: they failed to insert.
  (dolist (case '(("ka" . "ཀ") ("bkra" . "བཀྲ") ("phyw" . "ཕྱྭ") ("sku" . "སྐུ") ("ki" . "ཀི")))
    (should (equal (tibetan-tests--typed (car case)) (cdr case)))))

(ert-deftest tibetan-precomposed-transcriptions-are-unicode-stacks ()
  ;; Every transcription a stack has gives its stack, the 13 "br"/"bl"
  ;; ones GNU's lookup through the glyph table lost among them.
  (should (equal (tibetan-transcription-to-tibetan "brka") "བརྐ"))
  (dolist (alist (list tibetan-consonant-transcription-alist tibetan-vowel-transcription-alist
                       tibetan-modifier-transcription-alist tibetan-precomposed-transcription-alist
                       tibetan-subjoined-transcription-alist tibetan-base-to-subjoined-alist))
    (dolist (entry alist)
      (should-not (seq-some (lambda (c) (> c #x10FFFF)) (cdr entry))))))

(ert-deftest tibetan-transcription-round-trips-on-unicode ()
  (dolist (key '("phyw" "sku" "bkri" "rtsw"))
    (let ((text (tibetan-transcription-to-tibetan key)))
      (should (equal (tibetan-transcription-to-tibetan (tibetan-tibetan-to-transcription text)) text)))))

(ert-deftest tibetan-compose-and-decompose-round-trip-on-unicode ()
  (let ((text "བཀྲ་ཤིས་བདེ་ལེགས། ཕྱྭ་སྐུ"))
    (should (equal (tibetan-decompose-string (tibetan-compose-string (copy-sequence text))) text))
    (with-temp-buffer
      (insert text)
      (tibetan-compose-region (point-min) (point-max))
      (tibetan-decompose-region (point-min) (point-max))
      (should (equal (buffer-substring-no-properties (point-min) (point-max)) text))
      (should-not (text-property-not-all (point-min) (point-max) 'composition nil)))))

(ert-deftest tibetan-input-method-types-a-lone-vowel ()
  ;; The vowel "a" alone translates to nothing, which the update must take.
  (with-temp-buffer
    (activate-input-method "tibetan-wylie")
    (let ((quail-current-key "a") (quail-current-str ""))
      (should-not (quail-tibetan-update-translation nil))
      (should (equal quail-current-str "")))))

(defun tibetan-tests--keyed (method keys)
  "The text input method METHOD inserts for KEYS typed one by one, as its
input function receives them, with the package's guidance (on for both).
As the command loop, it gives the input method only printing characters:
a key left after a translation ends is dropped."
  (with-temp-buffer
    (switch-to-buffer (current-buffer))
    (activate-input-method method)
    (let ((input-method-verbose-flag t)
          (events (listify-key-sequence keys)))
      (while events
        (setq unread-command-events (cdr events))
        (let ((inserted (and (characterp (car events)) (<= ?\s (car events) 255) (/= (car events) 127)
                             (funcall input-method-function (car events)))))
          (setq events unread-command-events unread-command-events nil)
          (dolist (char inserted) (when (characterp char) (insert char)))))
      (buffer-substring-no-properties (point-min) (point-max)))))

(ert-deftest tibetan-input-methods-type-the-inherent-vowel-and-stacks ()
  ;; The inherent vowel ("" for Wylie `a', TibKey `A') and a stack are one
  ;; Quail candidate each: as many candidates as characters, none for "",
  ;; signalled `args-out-of-range'.  Pinned GNU types the same keys as
  ;; these, with its invisible vowel sign where the vowel is inherent.
  (dolist (case '(("tibetan-wylie" "ka " "ཀ་") ("tibetan-wylie" "a " "་") ("tibetan-wylie" "rka " "རྐ་")
                  ("tibetan-wylie" "phyw " "ཕྱྭ་") ("tibetan-tibkey" "qA " "ཀ་") ("tibetan-tibkey" "A " "་")))
    (should (equal (tibetan-tests--keyed (nth 0 case) (nth 1 case)) (nth 2 case)))))

(ert-deftest tibetan-wylie-bar-offers-each-special-sign ()
  ;; Wylie `|' offers the special signs, one candidate each: `C-f' moves to
  ;; the second and `C-SPC' takes it.  Given as one unit, `|' inserted
  ;; them all.  Pinned GNU types the same.
  (should (equal (tibetan-tests--keyed "tibetan-wylie" (kbd "| C-f C-SPC")) "༁")))

;;; tibetan-tests.el ends here
