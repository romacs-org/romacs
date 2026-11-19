;;; quail-big5-tests.el --- Big5 input methods offer Unicode only -*- lexical-binding: t; -*-

;; GNU's ARRAY30, ECDICT and Punct-b5 tables offered Big5 characters above
;; #x10FFFF, which no buffer here holds (ledger/divergences/VM2-BIG5-UNICODE.md):
;; the two duplicate ideographs are 十 and 卅, the eight ETEN box-drawing
;; characters are gone from a row that offers their Unicode forms already,
;; and the 30 unassigned A3C3..A3E0 codes are gone.

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

(ert-deftest quail-big5-punctuation-digits-keep-their-other-candidates ()
  (should (equal (mapcar (lambda (key) (apply #'concat (quail-big5-tests--candidates "chinese-punct-b5" key)))
                         '("0" "1" "2" "3" "4" "5" "6" "7" "8" "9"))
                 '("０Ⅹ十" "１Ⅰ〡卄" "２Ⅱ〢卅" "３Ⅲ〣" "４Ⅳ〤" "５Ⅴ〥" "６Ⅵ〦" "７Ⅶ〧" "８Ⅷ〨" "９Ⅸ〩"))))

;;; quail-big5-tests.el ends here
