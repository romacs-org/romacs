;;; romacs-raw-bytes-tests.el --- Raw bytes in multibyte buffers -*- lexical-binding: t; -*-

;; GNU's eight-bit characters (#x3FFF80..#x3FFFFF) in multibyte buffers
;; (DIVERGENCES.md VM2-RAWTEXT1, VM2-BUFNAME-RAW1, VM2-STR2).  Every
;; expected value below is pinned GNU Emacs 32.0.50's answer to the same
;; form (2026-10-30); the file passes under GNU itself:
;;   emacs -Q --batch -l romacs-raw-bytes-tests.el -f ert-run-tests-batch-and-exit

(require 'ert)

(defconst raw-bytes-test-e9 (string #x3fffe9)
  "The raw byte 0xE9 as a one-character multibyte string.")

(defun rb (unibyte)
  "The multibyte string whose bytes above 127 are UNIBYTE's as raw bytes.
A \"\\351\" literal is unibyte; buffer text and `buffer-string' are
multibyte, so expectations compare against this."
  (string-to-multibyte unibyte))

(ert-deftest raw-bytes-insert-positions-and-reads ()
  "A raw byte is one character of two bytes, read back as its code."
  (with-temp-buffer
    (insert "a\351b")
    (should (equal (list (char-after 1) (char-after 2) (char-after 3)) '(97 4194281 98)))
    (should (equal (mapcar #'position-bytes '(1 2 3 4)) '(1 2 4 5)))
    (should (equal (buffer-string) (rb "a\351b")))
    (should (multibyte-string-p (buffer-string)))
    (should (= (string-bytes (buffer-string)) 4))))

(ert-deftest raw-bytes-insertion-variants ()
  "Characters, unibyte strings, raw strings and inherit/marker variants."
  (with-temp-buffer
    (insert "\351" ?é (unibyte-string 233) #x3fffe9)
    (should (equal (buffer-string) (concat (rb "\351") "é" (rb "\351\351"))))
    (should (equal (mapcar #'char-after '(1 2 3 4)) '(4194281 233 4194281 4194281)))
    (should (= (position-bytes 5) 9)))
  (with-temp-buffer
    (insert "x")
    (let ((m (copy-marker 2)))
      (goto-char 2)
      (insert-before-markers "\351")
      (should (equal (list (buffer-string) (marker-position m)) (list (rb "x\351") 3)))))
  (with-temp-buffer
    (insert (string-to-multibyte (unibyte-string 233 65)))
    (should (equal (list (char-after 1) (char-after 2)) '(4194281 65))))
  (with-temp-buffer
    (insert (propertize "\351q" 'face 'bold))
    (should (equal (list (get-text-property 1 'face) (get-text-property 2 'face)) '(bold bold)))
    (should (equal (buffer-substring 1 3) (propertize (rb "\351q") 'face 'bold)))))

(ert-deftest raw-bytes-search-and-motion ()
  "A raw byte is not the Latin-1 character of its code; `.' matches it."
  (with-temp-buffer
    (insert "a\351bé")
    (goto-char 1)
    (should-not (search-forward "é" 2 t))
    (goto-char 1)
    (should (= (search-forward "é" nil t) 5))
    (goto-char 1)
    (should (= (re-search-forward "a.b" nil t) 4))
    (goto-char 1)
    (should (= (re-search-forward "[^[:ascii:]]" nil t) 3))
    (goto-char 1)
    (should (= (search-forward raw-bytes-test-e9 nil t) 3)))
  (with-temp-buffer
    (insert "ab\351cd")
    (goto-char 1)
    (should (equal (list (skip-chars-forward "a-z") (point)) '(2 3)))
    (should (equal (list (skip-chars-forward "^z") (point)) '(3 6)))))

(ert-deftest raw-bytes-columns-and-width ()
  "A raw byte shows as \\NNN: four columns."
  (with-temp-buffer
    (insert "A\351B")
    (should (= (current-column) 6))
    (goto-char 2)
    (should (= (current-column) 1))
    (should (= (progn (move-to-column 3) (point)) 3)))
  (should (equal (list (char-width #x3fffe9) (string-width "\351") (string-width (string #x3fffe9 ?a)))
                 '(4 1 5))))

(ert-deftest raw-bytes-casing-and-substitution ()
  "Casing leaves a raw byte; substitution keeps byte widths."
  (with-temp-buffer
    (buffer-enable-undo)
    (insert "a\351b")
    (undo-boundary)
    (upcase-region 1 4)
    (undo-boundary)
    (should (equal (buffer-string) (rb "A\351B")))
    (undo)
    (should (equal (buffer-string) (rb "a\351b"))))
  (with-temp-buffer
    (insert "\351")
    (should (equal (cadr (should-error (subst-char-in-region 1 2 #x3fffe9 ?z)))
                   "Characters in ‘subst-char-in-region’ have different byte-lengths")))
  (with-temp-buffer
    (insert "é\351")
    (subst-char-in-region 1 3 #x3fffe9 ?é)
    (subst-char-in-region 1 2 ?é #x3fffe9)
    (should (equal (list (buffer-string) (char-after 1) (char-after 2))
                   (list (concat (rb "\351") "é") 4194281 233)))))

(ert-deftest raw-bytes-coding-and-base64 ()
  "Decoding, encoding and base64 in place."
  (with-temp-buffer
    (insert "6Q==")
    (base64-decode-region 1 5)
    (should (equal (list (buffer-string) (char-after 1)) (list (rb "\351") 4194281))))
  (with-temp-buffer
    (insert "\303\251\351")
    (decode-coding-region 1 4 'utf-8)
    (should (equal (list (buffer-substring-no-properties 1 3) (char-after 1) (char-after 2))
                   (list (concat "é" (rb "\351")) 233 4194281))))
  (with-temp-buffer
    (insert "é\351")
    (encode-coding-region 1 3 'utf-8)
    (should (equal (string-to-list (buffer-string)) '(4194243 4194217 4194281))))
  (with-temp-buffer
    (insert "a\351")
    (should (equal (find-coding-systems-region 1 3) '(raw-text no-conversion))))
  (let ((file (make-temp-file "raw")))
    (unwind-protect
        (progn
          (let ((coding-system-for-write 'utf-8))
            (with-temp-buffer (insert "a\351é") (write-region 1 4 file)))
          (should (equal (with-temp-buffer
                           (set-buffer-multibyte nil)
                           (insert-file-contents-literally file)
                           (string-to-list (buffer-string)))
                         '(97 233 195 169))))
      (delete-file file))))

(ert-deftest raw-bytes-unibyte-conversions ()
  "set-buffer-multibyte in both directions, markers moved with the text."
  (with-temp-buffer
    (insert "A\351B")
    (set-buffer-multibyte nil)
    (should (equal (list (buffer-string) (point-max)) (list "A\351B" 4)))
    (set-buffer-multibyte t)
    (should (equal (list (buffer-string) (point-max) (char-after 2)) (list (rb "A\351B") 4 4194281))))
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert 233 "x" 195 169)
    (let ((m (point-marker)))
      (goto-char 2)
      (set-buffer-multibyte t)
      (should (equal (list (buffer-string) (point-max) (point) (marker-position m) (char-after 1) (char-after 3))
                     (list (concat (rb "\351") "xé") 4 2 4 4194281 233))))))

(ert-deftest raw-bytes-reader-indirect-and-messages ()
  "The buffer reader, indirect buffers and *Messages*."
  (with-temp-buffer
    (insert "\"a\351\" sym")
    (goto-char 1)
    ;; An octal escape reads as a unibyte string, in a buffer as elsewhere.
    (should (equal (list (read (current-buffer)) (point)) (list "a\351" 5))))
  (with-temp-buffer
    (insert "\351")
    (let ((b (make-indirect-buffer (current-buffer) " raw-bytes-indirect")))
      (unwind-protect (should (= (with-current-buffer b (char-after 1)) 4194281))
        (kill-buffer b))))
  (message "raw: \351x")
  (with-current-buffer "*Messages*"
    (goto-char (point-max))
    (should (search-backward (rb "raw: \351x") nil t))
    (should (equal (list (char-after (+ (point) 5))
                         (- (position-bytes (+ (point) 6)) (position-bytes (+ (point) 5))))
                   '(4194281 2)))))

(ert-deftest raw-bytes-strings-mixed ()
  "concat and format convert unibyte parts of a multibyte result."
  ;; Unibyte parts and an ASCII format give a unibyte result.
  (should (equal (format "a %s" (unibyte-string 233)) "a \351"))
  (should (equal (format "a %s é" (unibyte-string 233)) (concat (rb "a \351") " é")))
  (should (equal (concat "a" (unibyte-string 233) "é") (concat (rb "a\351") "é")))
  (should (equal (format "%s|%S" "\351" "\351") "\351|\"\\351\"")))

(ert-deftest raw-bytes-buffer-names ()
  "Buffer names hold raw bytes and round-trip through every name primitive."
  (let ((b (get-buffer-create (string ?a #x3fffe9))))
    (unwind-protect
        (progn
          (should (equal (buffer-name b) (rb "a\351")))
          (should (eq (get-buffer (string ?a #x3fffe9)) b))
          (should-not (get-buffer "aé"))
          (should (equal (format "%S %s" b b) "#<buffer a\\351> a\\351"))
          (should (equal (generate-new-buffer-name (string ?a #x3fffe9)) (rb "a\351<2>")))
          (with-current-buffer b (rename-buffer (string ?z #x3fff80)))
          (should (equal (buffer-name b) (rb "z\200"))))
      (kill-buffer b))))

(ert-deftest raw-bytes-symbols-and-file-names ()
  "Unibyte names intern as raw-byte symbols; file names with raw bytes name
the file whose name has those bytes."
  (should (equal (string-to-multibyte (symbol-name (intern (encode-coding-string (string ?a #x3fffe9) 'utf-8))))
                 (rb "a\351")))
  (let* ((dir (make-temp-file "raw-dir" t))
         (raw (concat (file-name-as-directory dir) raw-bytes-test-e9 "x.txt")))
    (unwind-protect
        (progn
          (with-temp-file raw (insert "raw name\n"))
          (should (equal (directory-files dir nil "txt") (list (concat raw-bytes-test-e9 "x.txt"))))
          (should (file-exists-p raw))
          (should (= (file-attribute-size (file-attributes raw)) 9))
          (should (equal (with-temp-buffer (insert-file-contents raw) (buffer-string)) "raw name\n"))
          (should (equal (file-name-all-completions (string #x3fffe9) dir)
                         (list (concat raw-bytes-test-e9 "x.txt"))))
          (rename-file raw (concat (file-name-as-directory dir) "renamed-" raw-bytes-test-e9 ".txt"))
          (should (equal (directory-files dir nil "txt")
                         (list (concat "renamed-" raw-bytes-test-e9 ".txt")))))
      (delete-directory dir t))))

(provide 'romacs-raw-bytes-tests)
;;; romacs-raw-bytes-tests.el ends here
