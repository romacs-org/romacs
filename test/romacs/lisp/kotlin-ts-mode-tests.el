;;; kotlin-ts-mode-tests.el --- Tests for Kotlin mode  -*- lexical-binding: t; -*-

(require 'ert)
(require 'ert-x)
(require 'cl-lib)
(require 'kotlin-ts-mode)

(defun kotlin-ts-mode-tests--position (text)
  "Return the position of the first occurrence of TEXT."
  (save-excursion
    (goto-char (point-min))
    (search-forward text)
    (match-beginning 0)))

(ert-deftest kotlin-ts-mode-selects-source-and-script ()
  (dolist (name '("Greeter.kt" "example.kts"))
    (let ((buffer (find-file-noselect (ert-resource-file name))))
      (unwind-protect
          (with-current-buffer buffer
            (should (eq major-mode 'kotlin-ts-mode))
            (let ((root (treesit-buffer-root-node 'kotlin)))
              (should (equal (treesit-node-type root) "source_file"))
              (should-not (treesit-node-check root 'has-error))))
        (kill-buffer buffer)))))

(ert-deftest kotlin-ts-mode-compiles-all-font-lock-queries ()
  (should (treesit-language-available-p 'kotlin))
  (should kotlin-ts-mode--font-lock-settings)
  (dolist (setting kotlin-ts-mode--font-lock-settings)
    (treesit-query-capture 'kotlin (treesit-font-lock-setting-query setting))))

(ert-deftest kotlin-ts-mode-font-lock ()
  (with-temp-buffer
    (insert-file-contents (ert-resource-file "Greeter.kt"))
    (let ((treesit-font-lock-level 4))
      (kotlin-ts-mode)
      (font-lock-ensure))
    (dolist (entry '(("class Greeter" . font-lock-keyword-face)
                     ("Greeter" . font-lock-type-face)
                     ("fun greet" . font-lock-keyword-face)
                     ("greet(name" . font-lock-function-name-face)
                     ("val prefix" . font-lock-keyword-face)
                     ("prefix =" . font-lock-variable-name-face)
                     ("var count" . font-lock-keyword-face)
                     ("${prefix}" . font-lock-delimiter-face)
                     ("Hello" . font-lock-string-face)
                     ("@Deprecated" . font-lock-preprocessor-face)
                     ("Deprecated" . font-lock-preprocessor-face)
                     ("// Count" . font-lock-comment-face)
                     ("nested" . font-lock-comment-face)
                     ("0" . font-lock-number-face)
                     ("true" . font-lock-constant-face)))
      (should (eq (get-text-property
                   (kotlin-ts-mode-tests--position (car entry)) 'face)
                  (cdr entry))))))

(ert-deftest kotlin-ts-mode-indents-blocks-and-continuations ()
  (with-temp-buffer
    (insert-file-contents (ert-resource-file "example.kts"))
    (let ((expected (buffer-string)))
      (goto-char (point-min))
      (while (re-search-forward "^ +" nil t)
        (replace-match ""))
      (kotlin-ts-mode)
      (indent-region (point-min) (point-max))
      (should (equal (buffer-string) expected)))))

(ert-deftest kotlin-ts-mode-navigates-and-indexes-declarations ()
  (with-temp-buffer
    (insert-file-contents (ert-resource-file "Greeter.kt"))
    (kotlin-ts-mode)
    (goto-char (kotlin-ts-mode-tests--position "// Count"))
    (should (beginning-of-defun))
    (should (bolp))
    (back-to-indentation)
    (should (looking-at-p "override fun greet"))
    (let ((index (funcall imenu-create-index-function)))
      (dolist (entry '(("Class" . "Greeter") ("Object" . "Examples")
                       ("Interface" . "Greeting") ("Function" . "greet")
                       ("Property" . "prefix")))
        (should (assoc (cdr entry) (cdr (assoc (car entry) index))))))))

(ert-deftest kotlin-ts-mode-syntax-comments-and-strings ()
  (with-temp-buffer
    (insert-file-contents (ert-resource-file "Greeter.kt"))
    (kotlin-ts-mode)
    (dolist (entry '(("nested" . 2) ("still outer" . 1)
                     ("// Count" . nil) ("Count each" . t)
                     ("interface" . nil)))
      (should (equal (nth 4 (syntax-ppss (kotlin-ts-mode-tests--position (car entry))))
                     (cdr entry))))
    (dolist (entry '(("Hello" . t) ("${prefix}" . t)
                     ("quoted" . t) ("a backslash" . t)
                     ("val empty" . nil) ("val trailingQuote" . nil)
                     ("val done" . nil)))
      (should (eq (nth 3 (syntax-ppss (kotlin-ts-mode-tests--position (car entry))))
                  (cdr entry))))
    (let* ((pos (kotlin-ts-mode-tests--position "literal text"))
           (state (syntax-ppss pos)))
      (should (nth 3 state))
      (should-not (nth 4 state)))
    (should (eq (nth 3 (syntax-ppss
                        (1+ (kotlin-ts-mode-tests--position "'\"'"))))
                ?'))))

(ert-deftest kotlin-ts-mode-syntax-escaped-identifiers ()
  (dolist (name '("odd\"name" "odd(name" "odd/*name"))
    (with-temp-buffer
      (insert "fun `" name "`() {}\nval following = 3\n")
      (kotlin-ts-mode)
      (let ((state (syntax-ppss (kotlin-ts-mode-tests--position "val following"))))
        (should (= (nth 0 state) 0))
        (should-not (nth 3 state))
        (should-not (nth 4 state))))))

(ert-deftest kotlin-ts-mode-syntax-after-nested-templates ()
  (dolist (literal '("\"before ${\"/*\"} after\""
                     "\"before ${\"}\"} after\""
                     "\"\"\"before ${\"\"\"}\"\"\"} after\"\"\""
                     "\"before $`has\"quote` after\""
                     "\"before \\${literal} and \\\" after\""
                     "\"before ${if (true) { \"}\" } else { /* } */ \"{\" }} after\""))
    (with-temp-buffer
      (insert "fun f() {\n    val text = " literal "\n    val following = 0\n}\n")
      (kotlin-ts-mode)
      (let ((state (syntax-ppss (kotlin-ts-mode-tests--position "val following"))))
        (should (= (nth 0 state) 1))
        (should-not (nth 3 state))
        (should-not (nth 4 state))))))

(ert-deftest kotlin-ts-mode-syntax-revisits-an-edited-multiline-template ()
  (with-temp-buffer
    (insert "fun f() {\n    val text = \"\"\"before ${listOf(\n"
            "        \"/*\"\n    )} after\"\"\"\n    val following = 0\n}\n")
    (kotlin-ts-mode)
    (cl-letf (((symbol-function 'treesit-parser-changed-regions)
               (lambda (&rest _) (ert-fail "Lexical scan requested a reparse"))))
      (let ((syntax-propertize-chunk-size 16))
        (should (nth 3 (syntax-ppss (kotlin-ts-mode-tests--position "before"))))
        (goto-char (kotlin-ts-mode-tests--position "\"/*\""))
        (delete-char 4)
        (insert "\"\"\"}\"\"\"")
        (let ((state (syntax-ppss (kotlin-ts-mode-tests--position "val following"))))
          (should (= (nth 0 state) 1))
          (should-not (nth 3 state))
          (should-not (nth 4 state)))))))

(ert-deftest kotlin-ts-mode-syntax-first-read-after-quote-edit ()
  (require 'elec-pair)
  (dolist (line '("val x = \"\"" "val x = \"\"\"raw\"\"\""))
    (dolist (reader '(kotlin-ts-mode-tests--electric-string-start
                      kotlin-ts-mode-tests--string-start))
      (with-temp-buffer
        (insert line "\nval following = 0\n")
        (kotlin-ts-mode)
        (syntax-ppss (point-max))
        (goto-char (1+ (length line)))
        (insert "\"")
        (let* ((pos (point))
               (opener (kotlin-ts-mode-tests--position "\""))
               (opening (equal line "val x = \"\"")))
          (should (equal (funcall reader (if opening pos (1- pos))) opener))
          (should (equal (kotlin-ts-mode-tests--string-start pos)
                         (and opening opener))))))))

(defun kotlin-ts-mode-tests--electric-string-start (pos)
  "Return the enclosing string or comment electric pairing sees at POS."
  (goto-char pos)
  (nth 3 (electric-pair-syntax-info ?\")))

(defun kotlin-ts-mode-tests--string-start (pos)
  "Return the enclosing string or comment reported by `syntax-ppss' at POS."
  (nth 8 (syntax-ppss pos)))

(ert-deftest kotlin-ts-mode-syntax-after-narrowed-raw-edit ()
  (dolist (case '(("fun f() {\n val s = \"\"\"before\n abc \" def\n ghi\n jkl\n \"\"\"\n val following = 0\n}\n"
                  "abc" "jkl" 0 "abc")
                 ("fun f() {\n val s = \"\"\"before\"\"\"\"\n val following = 0\n}\n"
                  nil "\"\"\"\"" 3 "before")))
    (pcase-let ((`(,source ,start ,stop ,offset ,edit) case))
      (dolist (warm '(nil t))
        (with-temp-buffer
          (insert source)
          (kotlin-ts-mode)
          (when warm (syntax-ppss (point-max)))
          (save-restriction
            (narrow-to-region
             (if start (kotlin-ts-mode-tests--position start) (point-min))
             (+ (kotlin-ts-mode-tests--position stop) offset))
            (goto-char (kotlin-ts-mode-tests--position edit))
            (insert "x")
            (syntax-ppss (point-max)))
          (let ((state (syntax-ppss
                        (kotlin-ts-mode-tests--position "val following"))))
            (should (= (nth 0 state) 1))
            (should-not (nth 3 state))
            (should-not (nth 4 state))))))))
