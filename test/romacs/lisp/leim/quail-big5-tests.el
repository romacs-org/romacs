;;; quail-big5-tests.el --- Big5 input methods offer Unicode only -*- lexical-binding: t; -*-

;; GNU's ARRAY30, ECDICT and Punct-b5 tables offered Big5 characters above
;; #x10FFFF, which no buffer here holds (ledger/divergences/VM2-BIG5-UNICODE.md):
;; the two duplicate ideographs are 十 and 卅, the eight ETEN box-drawing
;; characters are gone from a row that offers their Unicode forms already,
;; and the 30 control pictures A3C3..A3E0 are their Unicode forms, in place.

(require 'ert)
(require 'quail)

(defun quail-big5-tests--candidates (method key)
  "The candidates input METHOD offers for KEY, as a list of strings."
  (with-temp-buffer
    (activate-input-method method)
    (let ((translation (car (quail-lookup-key key))))
      (append (if (consp translation) (cdr translation) translation) nil))))

(defun quail-big5-tests--characters (method)
  "Every character input METHOD's map offers."
  (with-temp-buffer
    (activate-input-method method)
    (let ((seen (make-hash-table :test 'eq)) found)
      (cl-labels ((chars (tr)
                    (cond ((integerp tr) (list tr))
                          ((stringp tr) (append tr nil))
                          ((vectorp tr) (apply #'append (mapcar #'chars tr)))
                          ((and (consp tr) (vectorp (cdr tr))) (chars (cdr tr)))))
                  (walk (map)
                    (unless (gethash map seen)
                      (puthash map t seen)
                      (unless (functionp (car map))
                        (setq found (nconc (chars (car map)) found)))
                      (when (listp (cdr map))
                        (dolist (kv (cdr map))
                          (when (and (consp kv) (consp (cdr kv)))
                            (walk (cdr kv))))))))
        (walk (quail-map)))
      found)))

(ert-deftest quail-big5-methods-offer-unicode-only ()
  (dolist (method '("chinese-array30" "chinese-ecdict" "chinese-punct-b5"))
    (should (quail-big5-tests--characters method))
    (should-not (seq-some (lambda (c) (> c #x10FFFF)) (quail-big5-tests--characters method)))))

(ert-deftest quail-big5-duplicate-ideographs-are-unicode ()
  (should (equal (quail-big5-tests--candidates "chinese-ecdict" "0") '("０" "Ⅹ" "十")))
  (should (equal (quail-big5-tests--candidates "chinese-ecdict" "2") '("２" "Ⅱ" "〢" "卅")))
  (should (equal (last (quail-big5-tests--candidates "chinese-array30" "w8") 3) '("十" "卄" "卅"))))

(ert-deftest quail-big5-box-drawing-offers-each-once ()
  (let ((row (quail-big5-tests--candidates "chinese-array30" "w7")))
    (should (= (length row) 69))
    (dolist (box '("═" "╞" "╪" "╡" "╭" "╮" "╰" "╯"))
      (should (= (seq-count (lambda (c) (equal c box)) row) 1)))))

(ert-deftest quail-big5-punctuation-digits-keep-their-candidates-in-place ()
  ;; Each digit row offers its control pictures where GNU's Big5 codes were,
  ;; so the selection digits are GNU's.
  (should (equal (mapcar (lambda (key) (apply #'concat (quail-big5-tests--candidates "chinese-punct-b5" key)))
                         '("0" "1" "2" "9"))
                 '("０\N{U+240C}\N{U+2416}\N{U+2421}Ⅹ十"
                   "１\N{U+2403}\N{U+240D}\N{U+2417}Ⅰ〡卄"
                   "２\N{U+2404}\N{U+240E}\N{U+2418}Ⅱ〢卅"
                   "９\N{U+240B}\N{U+2415}\N{U+241F}Ⅸ〩"))))

(defun quail-big5-tests--keyed (method keys)
  "The text input method METHOD inserts for KEYS typed one by one."
  (with-temp-buffer
    (switch-to-buffer (current-buffer))
    (activate-input-method method)
    (let ((input-method-verbose-flag t) (events (listify-key-sequence keys)))
      (while events
        (setq unread-command-events (cdr events))
        (let ((inserted (funcall input-method-function (car events))))
          (setq events unread-command-events unread-command-events nil)
          (dolist (char inserted) (when (characterp char) (insert char)))))
      (buffer-substring-no-properties (point-min) (point-max)))))

(ert-deftest quail-big5-punctuation-selection-digits-are-gnus ()
  ;; Pinned GNU: 05 gives Ⅹ (U+2169), 15 Ⅰ, 25 Ⅱ, 06 its A2CC (here 十);
  ;; 02 its A3CC, here SYMBOL FOR FORM FEED (U+240C).
  (dolist (case '(("05" . "Ⅹ") ("15" . "Ⅰ") ("25" . "Ⅱ") ("06" . "十") ("02" . "\N{U+240C}")))
    (should (equal (quail-big5-tests--keyed "chinese-punct-b5" (car case)) (cdr case)))))

;;; quail-big5-tests.el ends here
