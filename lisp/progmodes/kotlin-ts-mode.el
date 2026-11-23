;;; kotlin-ts-mode.el --- Tree-sitter support for Kotlin  -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Major mode for Kotlin source files and scripts.

;;; Code:

(require 'treesit)
(eval-when-compile (require 'rx))

(defgroup kotlin-ts nil
  "Editing Kotlin with tree-sitter."
  :group 'languages)

(defcustom kotlin-ts-mode-indent-offset 4
  "Number of columns for a Kotlin block or continuation."
  :type 'natnum
  :safe #'natnump
  :group 'kotlin-ts)

(defvar kotlin-ts-mode-syntax-table
  (let ((table (make-syntax-table)))
    (dolist (char '(?+ ?- ?= ?% ?& ?| ?^ ?! ?< ?> ?@ ?$ ?`))
      (modify-syntax-entry char "." table))
    (modify-syntax-entry ?_ "_" table)
    (modify-syntax-entry ?' "\"" table)
    (modify-syntax-entry ?/ ". 124b" table)
    (modify-syntax-entry ?* ". 23n" table)
    (modify-syntax-entry ?\n "> b" table)
    table)
  "Syntax table used in Kotlin buffers.")

(defun kotlin-ts-mode--skip-code-token (end)
  "Advance over a comment, character or escaped identifier up to END."
  (save-restriction
    (narrow-to-region (point-min) end)
    (let ((parse-sexp-lookup-properties nil))
      (pcase (char-after)
        (?/
         (if (looking-at-p (rx "/" (any "/*")))
             (unless (forward-comment 1)
               (goto-char end))
           (forward-char)))
        (?'
         (goto-char (or (ignore-error scan-error (scan-sexps (point) 1)) end)))
        (?`
         (forward-char)
         (skip-chars-forward "^`\r\n")
         (when (eq (char-after) ?`)
           (forward-char)))))))

(defun kotlin-ts-mode--string-opener (end)
  "Skip a string opener up to END and return its kind."
  (if (looking-at-p "\"\"\"")
      (progn (goto-char (min end (+ (point) 3))) 'raw)
    (forward-char)
    'string))

(defun kotlin-ts-mode--scan-string (end)
  "Advance across a string through END and return whether it closed.
Track braces, comments, identifiers and nested strings in templates."
  (let ((states (list (kotlin-ts-mode--string-opener end))))
    (while (and states (< (point) end)
                (re-search-forward
                 (pcase (car states)
                   ('raw "[$\"]")
                   ('string (rx (or "$" "\"" (seq "\\" anything))))
                   ('code "[\"'`/{}]"))
                 end 'move))
      (let* ((start (match-beginning 0))
             (char (char-after start)))
        (if (eq (car states) 'code)
            (pcase char
              (?{ (push 'code states))
              (?} (pop states))
              (?\" (goto-char start)
                   (push (kotlin-ts-mode--string-opener end) states))
              (_ (goto-char start)
                 (kotlin-ts-mode--skip-code-token end)))
          (pcase char
            (?$
             (when (< (point) end)
               (pcase (char-after)
                 (?{ (forward-char) (push 'code states))
                 (?` (kotlin-ts-mode--skip-code-token end)))))
            (?\" (if (eq (car states) 'string)
                     (pop states)
                   (when (looking-at-p "\"\"")
                     (skip-chars-forward "\"" end)
                     (unless (eq (char-after) ?\")
                       (pop states)))))))))
    (null states)))

(defun kotlin-ts-mode--mark-literal (beg end)
  "Scan the token at point and mark its syntax within BEG and END."
  (let ((start (point)))
    (if (eq (char-after) ?\")
        (let ((closed (kotlin-ts-mode--scan-string end))
              (fence (string-to-syntax "|")))
          (when (>= start beg)
            (put-text-property start (1+ start) 'syntax-table fence))
          (when (and closed (> (point) beg))
            (put-text-property (1- (point)) (point) 'syntax-table fence)))
      (let ((identifier (eq (char-after) ?`)))
        (kotlin-ts-mode--skip-code-token end)
        (when identifier
          (add-text-properties
           start (point) (list 'syntax-table (string-to-syntax "_")
                               'syntax-multiline t)))))))

(defun kotlin-ts-mode--syntax-extend-region (beg end)
  "Move BEG back through a partially propertized identifier or comment opener."
  (when (> beg (point-min))
    (cond
     ((get-text-property (1- beg) 'syntax-multiline)
      (cons (previous-single-property-change beg 'syntax-multiline
                                             nil (point-min))
            end))
     ((and (eq (char-before beg) ?/) (memq (char-after beg) '(?/ ?*)))
      (cons (1- beg) end)))))

(defun kotlin-ts-mode--syntax-before-change (beg _end)
  "Invalidate the three-character syntax lookahead before an edit at BEG.
Electric pairing can query BEG itself, before region extension runs."
  (syntax-ppss-flush-cache (max (point-min) (- beg 3))))

(defun kotlin-ts-mode--syntax-propertize (beg end)
  "Set literal syntax within BEG and END using the widened syntax state."
  (let ((clipped (and (buffer-narrowed-p) (= beg (point-min)))))
    (save-restriction
      (widen)
      (when clipped
        ;; The shared propertizer may have skipped invalidated text before BEG.
        (remove-text-properties (point-min) beg '(syntax-table nil syntax-multiline nil))
        (setq beg (point-min))
        (syntax-ppss-flush-cache beg))
      (goto-char (or (nth 8 (syntax-ppss beg)) beg))
      (while (and (< (point) end) (re-search-forward "[\"'`/]" end t))
        (goto-char (match-beginning 0))
        (kotlin-ts-mode--mark-literal beg end)))))

(defconst kotlin-ts-mode--indent-rules
  `((kotlin
     ((parent-is "multiline_string_literal") no-indent 0)
     ((parent-is "block_comment") no-indent 0)
     ((node-is ,(rx bos (any "]})") eos)) parent-bol 0)
     ((match ,(rx bos (or "else" "block") eos) ,(rx bos "if_expression" eos))
      parent-bol 0)
     ((parent-is ,(rx bos (or "class_body" "enum_class_body" "block"
                              "lambda_literal" "when_expression" "when_entry"
                              "if_expression") eos))
      parent-bol kotlin-ts-mode-indent-offset)
     ((parent-is ,(rx bos (or "class_parameters" "function_value_parameters"
                              "value_arguments" "collection_literal"
                              "index_expression" "parenthesized_expression") eos))
      parent-bol kotlin-ts-mode-indent-offset)
     ((parent-is ,(rx bos (or "function_body" "property_declaration"
                              "binary_expression" "navigation_expression") eos))
      parent-bol kotlin-ts-mode-indent-offset)
     ((parent-is "source_file") column-0 0)))
  "Indent rules for Kotlin blocks and continuations.")

(defvar kotlin-ts-mode--font-lock-settings
  (treesit-font-lock-rules
   :default-language 'kotlin
   :feature 'comment
   '([(line_comment) (block_comment) (shebang)] @font-lock-comment-face)
   :feature 'string
   '([(string_content) (character_literal) (escape_sequence)
      "\"" "\"\"\"" "\"\"\"\""] @font-lock-string-face)
   :feature 'keyword
   '(["package" "import" "class" "interface" "object" "typealias" "fun"
      "val" "var" "constructor" "init" "companion" "where" "by"
      "if" "else" "when" "for" "while" "do" "try" "catch" "finally"
      "return" "throw" "in" "is" "as" "as?"
      "this" "super" "get" "set"
      (class_modifier) (function_modifier) (inheritance_modifier)
      (member_modifier) (parameter_modifier) (platform_modifier)
      (property_modifier) (visibility_modifier)] @font-lock-keyword-face
     ((identifier) @font-lock-keyword-face
      (:match "\\`\\(?:break\\|continue\\)\\'" @font-lock-keyword-face)))
   :feature 'type
   '((class_declaration name: (identifier) @font-lock-type-face)
     (object_declaration name: (identifier) @font-lock-type-face)
     (type_alias type: (identifier) @font-lock-type-face)
     (user_type (identifier) @font-lock-type-face))
   :feature 'function
   '((call_expression :anchor (identifier) @font-lock-function-call-face)
     (call_expression :anchor (navigation_expression (identifier) @font-lock-function-call-face :anchor)))
   :feature 'definition
   '((function_declaration name: (identifier) @font-lock-function-name-face)
     (variable_declaration (identifier) @font-lock-variable-name-face)
     (parameter (identifier) @font-lock-variable-name-face)
     (class_parameter (identifier) @font-lock-variable-name-face))
   :feature 'constant
   '(((identifier) @font-lock-constant-face
      (:match "\\`\\(?:true\\|false\\|null\\)\\'" @font-lock-constant-face)))
   :feature 'number
   '([(number_literal) (float_literal)] @font-lock-number-face)
   :feature 'operator
   '(["+" "-" "*" "/" "%" "=" "+=" "-=" "*=" "/=" "%="
      "==" "!=" "===" "!==" "<" ">" "<=" ">=" "&&" "||"
      "!" "!!" "++" "--" "!in" "!is" "->" "?:" "?." ".." "..<"]
     @font-lock-operator-face)
   :feature 'bracket
   '(["(" ")" "[" "]" "{" "}"] @font-lock-bracket-face)
   :feature 'delimiter
   :override t
   '(["," "." ":" ";"] @font-lock-delimiter-face
     (interpolation ["$" "${" "}"] @font-lock-delimiter-face))
   :feature 'annotation
   :override t
   '((annotation "@" @font-lock-preprocessor-face)
     (annotation (user_type (identifier) @font-lock-preprocessor-face))
     (annotation (constructor_invocation
                  (user_type (identifier) @font-lock-preprocessor-face)))))
  "Tree-sitter font-lock rules for Kotlin.")

(defconst kotlin-ts-mode--class-regexp "\\`class_declaration\\'"
  "Match the node type shared by class and interface declarations.")

(defun kotlin-ts-mode--interface-p (node)
  "Return non-nil if class declaration NODE declares an interface."
  (treesit-search-subtree node "\\`interface\\'" nil t 1))

(defun kotlin-ts-mode--class-p (node)
  "Return non-nil if class declaration NODE declares a class."
  (not (kotlin-ts-mode--interface-p node)))

(defun kotlin-ts-mode--defun-name (node)
  "Return the name of Kotlin declaration NODE."
  (let ((name (or (treesit-node-child-by-field-name node "name")
                  (when (equal (treesit-node-type node) "property_declaration")
                    (when-let* ((variable (treesit-search-subtree
                                           node "\\`variable_declaration\\'" nil nil 1)))
                      (treesit-search-subtree variable "\\`identifier\\'" nil nil 1))))))
    (when name (treesit-node-text name t))))

(defconst kotlin-ts-mode--imenu-settings
  `(("Class" ,kotlin-ts-mode--class-regexp kotlin-ts-mode--class-p nil)
    ("Interface" ,kotlin-ts-mode--class-regexp kotlin-ts-mode--interface-p nil)
    ("Object" "\\`object_declaration\\'" nil nil)
    ("Function" "\\`function_declaration\\'" nil nil)
    ("Property" "\\`property_declaration\\'" nil nil))
  "Imenu groups for Kotlin declarations.")

;;;###autoload
(define-derived-mode kotlin-ts-mode prog-mode "Kotlin"
  "Major mode for editing Kotlin using tree-sitter."
  (setq-local comment-start "// "
              comment-end ""
              comment-start-skip "//+\\s-*\\|/\\*+\\s-*"
              syntax-propertize-function #'kotlin-ts-mode--syntax-propertize
              syntax-propertize-extend-region-functions
              '(kotlin-ts-mode--syntax-extend-region)
              indent-tabs-mode nil)
  (add-hook 'before-change-functions #'kotlin-ts-mode--syntax-before-change nil t)
  (when (and (treesit-ensure-installed 'kotlin)
             (treesit-ready-p 'kotlin))
    (treesit-parser-create 'kotlin)
    (setq-local treesit-simple-indent-rules kotlin-ts-mode--indent-rules
                treesit-font-lock-settings kotlin-ts-mode--font-lock-settings
                treesit-font-lock-feature-list
                '((comment definition)
                  (keyword string type annotation)
                  (constant function number)
                  (operator bracket delimiter))
                treesit-defun-type-regexp
                (regexp-opt '("class_declaration" "object_declaration"
                              "function_declaration" "property_declaration") 'symbols)
                treesit-defun-name-function #'kotlin-ts-mode--defun-name
                treesit-simple-imenu-settings kotlin-ts-mode--imenu-settings)
    (treesit-major-mode-setup)
    (remove-hook 'syntax-propertize-extend-region-functions
                 #'treesit--pre-syntax-ppss t)))

;;;###autoload
(defun kotlin-ts-mode-maybe ()
  "Enable `kotlin-ts-mode' when its grammar is available.
Also offer to install the grammar when `treesit-enabled-modes' enables
the mode, subject to `treesit-auto-install-grammar'."
  (if (or (treesit-language-available-p 'kotlin)
          (eq treesit-enabled-modes t)
          (memq 'kotlin-ts-mode treesit-enabled-modes))
      (kotlin-ts-mode)
    (fundamental-mode)))

;;;###autoload
(when (boundp 'treesit-major-mode-remap-alist)
  (add-to-list 'auto-mode-alist '("\\.kts?\\'" . kotlin-ts-mode-maybe)))

(provide 'kotlin-ts-mode)
;;; kotlin-ts-mode.el ends here
