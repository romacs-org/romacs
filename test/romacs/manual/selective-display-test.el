;;; selective-display-test.el --- Test selective display, outline, and code folding -*- lexical-binding: t -*-

;; Test selective-display, outline-mode, hs-minor-mode, invisible text
;; properties, and code folding interactions with the romacs GPU renderer.
;; Verifies that hidden text, ellipsis indicators, line numbers, and cursor
;; navigation all render correctly through folded regions.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/selective-display-test.el

;;; Code:

(require 'outline)
(require 'hideshow)

(defvar sdt--test-step 0
  "Current test step index.")

(defvar sdt--buffers nil
  "List of test buffers created during tests.")

(defun sdt--cleanup ()
  "Kill all test buffers."
  (dolist (b sdt--buffers)
    (when (buffer-live-p b)
      (kill-buffer b)))
  (setq sdt--buffers nil))

(defun sdt--make-buffer (name)
  "Create and return a test buffer named NAME, tracking it for cleanup."
  (let ((buf (get-buffer-create (format "*sdt-%s*" name))))
    (push buf sdt--buffers)
    buf))

(defun sdt--insert-heading (text)
  "Insert a highlighted section heading TEXT."
  (let ((start (point)))
    (insert (format "\n=== %s ===\n\n" text))
    (put-text-property start (point) 'face '(:foreground "gold" :weight bold))))

(defun sdt--insert-hint (text)
  "Insert a hint/instruction line TEXT."
  (let ((start (point)))
    (insert (format "  %s\n" text))
    (put-text-property start (point) 'face '(:foreground "dark gray" :slant italic))))

;; ============================================================================
;; Test 1: selective-display at various indentation levels
;; ============================================================================

(defun sdt--test-selective-display ()
  "Test `selective-display' set to various indentation thresholds.
Lines indented more than the threshold should be hidden with `...' ellipsis."
  (let ((buf (sdt--make-buffer "selective-display")))
    (switch-to-buffer buf)
    (erase-buffer)
    (sdt--insert-heading "SELECTIVE-DISPLAY TEST")
    (insert "Lines indented beyond `selective-display' are hidden with \"...\" ellipsis.\n")
    (insert "Use the keybindings below to set different levels and observe changes.\n\n")
    (sdt--insert-hint "C-c 0   -- set-selective-display nil (show all)")
    (sdt--insert-hint "C-c 2   -- set-selective-display 2")
    (sdt--insert-hint "C-c 4   -- set-selective-display 4")
    (sdt--insert-hint "C-c 8   -- set-selective-display 8")
    (insert "\n")

    ;; Sample code with varying indentation
    (insert "top-level line A\n")
    (insert "  indented 2: line B\n")
    (insert "    indented 4: line C\n")
    (insert "      indented 6: line D\n")
    (insert "        indented 8: line E\n")
    (insert "          indented 10: line F\n")
    (insert "top-level line G\n")
    (insert "  indented 2: line H\n")
    (insert "    indented 4: line I\n")
    (insert "      indented 6: line J\n")
    (insert "top-level line K\n")
    (insert "\n")

    ;; A function-like structure
    (insert "function outer() {\n")
    (insert "  var x = 1;\n")
    (insert "  if (x > 0) {\n")
    (insert "    var y = 2;\n")
    (insert "    if (y > 1) {\n")
    (insert "      var z = 3;\n")
    (insert "      while (z > 0) {\n")
    (insert "        z = z - 1;\n")
    (insert "        if (z == 1) {\n")
    (insert "          print(\"deep\");\n")
    (insert "        }\n")
    (insert "      }\n")
    (insert "    }\n")
    (insert "  }\n")
    (insert "}\n")
    (insert "\n")

    ;; Another block
    (insert "class MyClass {\n")
    (insert "  constructor() {\n")
    (insert "    this.value = 0;\n")
    (insert "  }\n")
    (insert "  method_a() {\n")
    (insert "    return this.value + 1;\n")
    (insert "  }\n")
    (insert "  method_b() {\n")
    (insert "    if (this.value > 0) {\n")
    (insert "      return true;\n")
    (insert "    }\n")
    (insert "    return false;\n")
    (insert "  }\n")
    (insert "}\n")

    ;; Display line numbers for easier verification
    (display-line-numbers-mode 1)

    ;; Keybindings
    (local-set-key (kbd "C-c 0")
                   (lambda () (interactive)
                     (set-selective-display nil)
                     (message "selective-display: nil (all visible)")))
    (local-set-key (kbd "C-c 2")
                   (lambda () (interactive)
                     (set-selective-display 2)
                     (message "selective-display: 2 (indent > 2 hidden)")))
    (local-set-key (kbd "C-c 4")
                   (lambda () (interactive)
                     (set-selective-display 4)
                     (message "selective-display: 4 (indent > 4 hidden)")))
    (local-set-key (kbd "C-c 8")
                   (lambda () (interactive)
                     (set-selective-display 8)
                     (message "selective-display: 8 (indent > 8 hidden)")))

    (goto-char (point-min))
    ;; Start with level 4 so folding is visible immediately
    (set-selective-display 4)
    (message "Test 1: selective-display set to 4. Use C-c 0/2/4/8 to change.")))

;; ============================================================================
;; Test 2: outline-mode with heading levels and folding
;; ============================================================================

(defun sdt--test-outline-mode ()
  "Test `outline-mode' heading visibility cycling and nested folding."
  (let ((buf (sdt--make-buffer "outline-mode")))
    (switch-to-buffer buf)
    (erase-buffer)
    (outline-mode)

    (insert "* Top-Level Heading 1\n")
    (insert "\n")
    (insert "This is body text under heading 1.\n")
    (insert "It should be hideable via outline commands.\n")
    (insert "\n")
    (insert "** Sub-Heading 1.1\n")
    (insert "\n")
    (insert "Body text under sub-heading 1.1.\n")
    (insert "More content here that should fold.\n")
    (insert "\n")
    (insert "*** Sub-Sub-Heading 1.1.1\n")
    (insert "\n")
    (insert "Deeply nested body text under 1.1.1.\n")
    (insert "This is the innermost level of nesting.\n")
    (insert "Line A.\n")
    (insert "Line B.\n")
    (insert "Line C.\n")
    (insert "\n")
    (insert "** Sub-Heading 1.2\n")
    (insert "\n")
    (insert "Body text under sub-heading 1.2.\n")
    (insert "Another paragraph of body content.\n")
    (insert "\n")
    (insert "*** Sub-Sub-Heading 1.2.1\n")
    (insert "\n")
    (insert "Body under 1.2.1 with some detail.\n")
    (insert "\n")
    (insert "*** Sub-Sub-Heading 1.2.2\n")
    (insert "\n")
    (insert "Body under 1.2.2 with more detail.\n")
    (insert "Extra line.\n")
    (insert "\n")
    (insert "* Top-Level Heading 2\n")
    (insert "\n")
    (insert "Body text under heading 2.\n")
    (insert "Some explanation goes here.\n")
    (insert "\n")
    (insert "** Sub-Heading 2.1\n")
    (insert "\n")
    (insert "Body under 2.1.\n")
    (insert "Continuation line.\n")
    (insert "\n")
    (insert "** Sub-Heading 2.2\n")
    (insert "\n")
    (insert "Body under 2.2.\n")
    (insert "\n")
    (insert "* Top-Level Heading 3\n")
    (insert "\n")
    (insert "Final top-level section.\n")
    (insert "Body text for the third section.\n")
    (insert "Last line of the document.\n")

    (display-line-numbers-mode 1)
    (goto-char (point-min))

    ;; Fold all bodies initially to show only headings
    (outline-hide-body)

    (message "Test 2: outline-mode. TAB on heading to cycle, C-c C-a show all, C-c C-t hide body.")))

;; ============================================================================
;; Test 3: outline-minor-mode in Emacs Lisp
;; ============================================================================

(defun sdt--test-outline-minor-mode ()
  "Test `outline-minor-mode' with Emacs Lisp section comments."
  (let ((buf (sdt--make-buffer "outline-minor-elisp")))
    (switch-to-buffer buf)
    (erase-buffer)
    (emacs-lisp-mode)
    (outline-minor-mode 1)
    (setq-local outline-regexp ";;;+ ")

    (insert ";;; Section 1 -- Data Structures\n")
    (insert "\n")
    (insert "(defvar my-list '(1 2 3 4 5)\n")
    (insert "  \"A sample list.\")\n")
    (insert "\n")
    (insert "(defvar my-table (make-hash-table :test 'equal)\n")
    (insert "  \"A sample hash table.\")\n")
    (insert "\n")
    (insert "(defun my-list-length (lst)\n")
    (insert "  \"Return length of LST.\"\n")
    (insert "  (length lst))\n")
    (insert "\n")
    (insert ";;;; Subsection 1.1 -- List Utilities\n")
    (insert "\n")
    (insert "(defun my-filter (pred lst)\n")
    (insert "  \"Filter LST by PRED.\"\n")
    (insert "  (seq-filter pred lst))\n")
    (insert "\n")
    (insert "(defun my-map (fn lst)\n")
    (insert "  \"Map FN over LST.\"\n")
    (insert "  (mapcar fn lst))\n")
    (insert "\n")
    (insert ";;;; Subsection 1.2 -- Hash Utilities\n")
    (insert "\n")
    (insert "(defun my-hash-keys (table)\n")
    (insert "  \"Return keys of TABLE.\"\n")
    (insert "  (hash-table-keys table))\n")
    (insert "\n")
    (insert ";;; Section 2 -- String Operations\n")
    (insert "\n")
    (insert "(defun my-upcase (s)\n")
    (insert "  \"Upcase string S.\"\n")
    (insert "  (upcase s))\n")
    (insert "\n")
    (insert "(defun my-join (sep lst)\n")
    (insert "  \"Join LST with SEP.\"\n")
    (insert "  (string-join lst sep))\n")
    (insert "\n")
    (insert "(defun my-trim (s)\n")
    (insert "  \"Trim whitespace from S.\"\n")
    (insert "  (string-trim s))\n")
    (insert "\n")
    (insert ";;; Section 3 -- Math\n")
    (insert "\n")
    (insert "(defun my-factorial (n)\n")
    (insert "  \"Compute factorial of N.\"\n")
    (insert "  (if (<= n 1) 1\n")
    (insert "    (* n (my-factorial (1- n)))))\n")
    (insert "\n")
    (insert "(defun my-fibonacci (n)\n")
    (insert "  \"Compute Nth Fibonacci number.\"\n")
    (insert "  (if (< n 2) n\n")
    (insert "    (+ (my-fibonacci (- n 1))\n")
    (insert "       (my-fibonacci (- n 2)))))\n")
    (insert "\n")
    (insert ";;; End\n")

    (font-lock-ensure)
    (display-line-numbers-mode 1)
    (goto-char (point-min))

    ;; Fold to show only section headings
    (outline-hide-body)

    (message "Test 3: outline-minor-mode in Emacs Lisp. C-c @ C-a show all, C-c @ C-t hide body.")))

;; ============================================================================
;; Test 4: hs-minor-mode (hideshow) for code folding
;; ============================================================================

(defun sdt--test-hideshow ()
  "Test `hs-minor-mode' folding of functions, blocks, and comments."
  (let ((buf (sdt--make-buffer "hideshow")))
    (switch-to-buffer buf)
    (erase-buffer)
    (emacs-lisp-mode)
    (hs-minor-mode 1)

    (insert ";; HideShow (hs-minor-mode) Test\n")
    (insert ";;\n")
    (insert ";; Keybindings:\n")
    (insert ";;   C-c @ C-h  -- hs-hide-block     (fold block at point)\n")
    (insert ";;   C-c @ C-s  -- hs-show-block     (unfold block at point)\n")
    (insert ";;   C-c @ C-M-h -- hs-hide-all      (fold everything)\n")
    (insert ";;   C-c @ C-M-s -- hs-show-all      (unfold everything)\n")
    (insert ";;   C-c @ C-l  -- hs-hide-level     (fold to level)\n")
    (insert ";;   C-c @ C-c  -- hs-toggle-hiding  (toggle fold at point)\n")
    (insert "\n")

    ;; Simple function
    (insert "(defun simple-function (x)\n")
    (insert "  \"A simple function that doubles X.\"\n")
    (insert "  (* x 2))\n")
    (insert "\n")

    ;; Function with nested forms
    (insert "(defun complex-function (x y)\n")
    (insert "  \"A complex function with nested conditionals.\"\n")
    (insert "  (let ((result 0))\n")
    (insert "    (cond\n")
    (insert "     ((> x y)\n")
    (insert "      (setq result (- x y))\n")
    (insert "      (when (> result 100)\n")
    (insert "        (setq result 100)))\n")
    (insert "     ((< x y)\n")
    (insert "      (setq result (- y x))\n")
    (insert "      (when (< result 0)\n")
    (insert "        (setq result 0)))\n")
    (insert "     (t\n")
    (insert "      (setq result (+ x y))))\n")
    (insert "    result))\n")
    (insert "\n")

    ;; Multi-line comment block
    (insert ";; This is a long comment block that can be folded.\n")
    (insert ";; It spans multiple lines and explains the purpose\n")
    (insert ";; of the following code section. The hideshow mode\n")
    (insert ";; should be able to fold comment blocks too.\n")
    (insert ";; Line 5 of the comment block.\n")
    (insert "\n")

    ;; Interactive function
    (insert "(defun interactive-function ()\n")
    (insert "  \"An interactive function with user input.\"\n")
    (insert "  (interactive)\n")
    (insert "  (let ((name (read-string \"Name: \")))\n")
    (insert "    (message \"Hello, %s!\" name)\n")
    (insert "    (when (string-empty-p name)\n")
    (insert "      (message \"No name provided\"))))\n")
    (insert "\n")

    ;; Macro
    (insert "(defmacro with-timing (label &rest body)\n")
    (insert "  \"Execute BODY and report timing under LABEL.\"\n")
    (insert "  `(let ((start-time (current-time)))\n")
    (insert "     (prog1 (progn ,@body)\n")
    (insert "       (message \"%s took %.3fs\" ,label\n")
    (insert "                (float-time (time-subtract (current-time) start-time))))))\n")
    (insert "\n")

    ;; Nested defuns
    (insert "(defun outer-fn ()\n")
    (insert "  \"Outer function containing a local function.\"\n")
    (insert "  (cl-labels ((inner-fn (n)\n")
    (insert "                (if (zerop n) 1\n")
    (insert "                  (* n (inner-fn (1- n))))))\n")
    (insert "    (inner-fn 10)))\n")
    (insert "\n")

    ;; Final simple function
    (insert "(defun last-function ()\n")
    (insert "  \"The last function in the file.\"\n")
    (insert "  (message \"done\"))\n")
    (insert "\n")
    (insert ";;; End of hideshow test.\n")

    (font-lock-ensure)
    (display-line-numbers-mode 1)
    (goto-char (point-min))

    ;; Fold all blocks
    (hs-hide-all)

    (message "Test 4: hs-minor-mode. C-c @ C-c to toggle, C-c @ C-M-s show all, C-c @ C-M-h hide all.")))

;; ============================================================================
;; Test 5: invisible text property with and without ellipsis
;; ============================================================================

(defun sdt--test-invisible-property ()
  "Test the `invisible' text property with various `buffer-invisibility-spec' settings."
  (let ((buf (sdt--make-buffer "invisible-property")))
    (switch-to-buffer buf)
    (erase-buffer)
    (sdt--insert-heading "INVISIBLE TEXT PROPERTY TEST")
    (insert "Testing the `invisible' text property with different ellipsis settings.\n\n")

    ;; Configure GNU's buffer-invisibility-spec:
    ;; - `hidden-no-ellipsis' : invisible, no ellipsis
    ;; - `hidden-with-ellipsis' : invisible, shows "..." ellipsis
    ;; - `hidden-custom' : invisible, shows "..." ellipsis (t)
    (setq-local buffer-invisibility-spec
                '((hidden-no-ellipsis . nil)
                  (hidden-with-ellipsis . t)
                  (hidden-custom . t)))

    ;; --- Invisible without ellipsis ---
    (let ((start (point)))
      (insert "  [visible-A]")
      (put-text-property start (point) 'face '(:foreground "cyan")))
    (let ((start (point)))
      (insert "[THIS TEXT IS INVISIBLE WITHOUT ELLIPSIS]")
      (put-text-property start (point) 'invisible 'hidden-no-ellipsis))
    (let ((start (point)))
      (insert "[visible-B]\n")
      (put-text-property start (point) 'face '(:foreground "cyan")))
    (insert "  ^ Above: visible-A and visible-B should be adjacent, no gap or dots.\n\n")

    ;; --- Invisible with ellipsis ---
    (let ((start (point)))
      (insert "  [visible-C]")
      (put-text-property start (point) 'face '(:foreground "lime green")))
    (let ((start (point)))
      (insert "[THIS TEXT IS INVISIBLE WITH ELLIPSIS]")
      (put-text-property start (point) 'invisible 'hidden-with-ellipsis))
    (let ((start (point)))
      (insert "[visible-D]\n")
      (put-text-property start (point) 'face '(:foreground "lime green")))
    (insert "  ^ Above: visible-C, then \"...\", then visible-D.\n\n")

    ;; --- Multiple invisible spans on one line ---
    (insert "  Multiple hidden spans: ")
    (let ((start (point)))
      (insert "AAA")
      (put-text-property start (point) 'face '(:foreground "orange")))
    (let ((start (point)))
      (insert "[hidden1]")
      (put-text-property start (point) 'invisible 'hidden-with-ellipsis))
    (let ((start (point)))
      (insert "BBB")
      (put-text-property start (point) 'face '(:foreground "orange")))
    (let ((start (point)))
      (insert "[hidden2]")
      (put-text-property start (point) 'invisible 'hidden-no-ellipsis))
    (let ((start (point)))
      (insert "CCC")
      (put-text-property start (point) 'face '(:foreground "orange")))
    (let ((start (point)))
      (insert "[hidden3]")
      (put-text-property start (point) 'invisible 'hidden-with-ellipsis))
    (let ((start (point)))
      (insert "DDD\n")
      (put-text-property start (point) 'face '(:foreground "orange")))
    (insert "  ^ Should read: AAA...BBB CCC...DDD (two ellipses, one seamless join)\n\n")

    ;; --- Multi-line invisible region ---
    (insert "  Before multi-line hidden region:\n")
    (let ((start (point)))
      (insert "  HIDDEN LINE 1\n")
      (insert "  HIDDEN LINE 2\n")
      (insert "  HIDDEN LINE 3\n")
      (insert "  HIDDEN LINE 4\n")
      (insert "  HIDDEN LINE 5\n")
      (put-text-property start (point) 'invisible 'hidden-with-ellipsis))
    (insert "  After multi-line hidden region.\n")
    (insert "  ^ Five lines above should collapse to a single \"...\" between Before/After.\n\n")

    ;; --- Invisible region at beginning of line ---
    (let ((start (point)))
      (insert "INVISIBLE PREFIX: ")
      (put-text-property start (point) 'invisible 'hidden-with-ellipsis))
    (insert "visible remainder of line\n")
    (insert "  ^ Should show: ...visible remainder of line\n\n")

    ;; --- Invisible region at end of line ---
    (insert "  visible start of line")
    (let ((start (point)))
      (insert " :INVISIBLE SUFFIX")
      (put-text-property start (point) 'invisible 'hidden-with-ellipsis))
    (insert "\n")
    (insert "  ^ Should show: visible start of line...\n\n")

    ;; --- Toggle visibility keybindings ---
    (sdt--insert-hint "C-c v   -- make hidden-no-ellipsis visible (toggle)")
    (sdt--insert-hint "C-c e   -- make hidden-with-ellipsis visible (toggle)")
    (sdt--insert-hint "C-c a   -- show everything")
    (sdt--insert-hint "C-c h   -- hide everything again")

    (let ((all-visible nil)
          (no-ellipsis-visible nil)
          (with-ellipsis-visible nil))
      (local-set-key (kbd "C-c v")
                     (lambda () (interactive)
                       (setq no-ellipsis-visible (not no-ellipsis-visible))
                       (if no-ellipsis-visible
                           (remove-from-invisibility-spec '(hidden-no-ellipsis . nil))
                         (add-to-invisibility-spec '(hidden-no-ellipsis . nil)))
                       (message "hidden-no-ellipsis: %s"
                                (if no-ellipsis-visible "VISIBLE" "HIDDEN"))))
      (local-set-key (kbd "C-c e")
                     (lambda () (interactive)
                       (setq with-ellipsis-visible (not with-ellipsis-visible))
                       (if with-ellipsis-visible
                           (remove-from-invisibility-spec '(hidden-with-ellipsis . t))
                         (add-to-invisibility-spec '(hidden-with-ellipsis . t)))
                       (message "hidden-with-ellipsis: %s"
                                (if with-ellipsis-visible "VISIBLE" "HIDDEN"))))
      (local-set-key (kbd "C-c a")
                     (lambda () (interactive)
                       (setq-local buffer-invisibility-spec nil)
                       (message "All text visible")))
      (local-set-key (kbd "C-c h")
                     (lambda () (interactive)
                       (setq-local buffer-invisibility-spec
                                   '((hidden-no-ellipsis . nil)
                                     (hidden-with-ellipsis . t)
                                     (hidden-custom . t)))
                       (message "Invisibility restored"))))

    (display-line-numbers-mode 1)
    (goto-char (point-min))
    (message "Test 5: invisible text property. C-c v/e toggle, C-c a show all, C-c h re-hide.")))

;; ============================================================================
;; Test 6: Multiple levels of outline folding (nested)
;; ============================================================================

(defun sdt--test-nested-outline ()
  "Test multiple nested outline folding levels."
  (let ((buf (sdt--make-buffer "nested-outline")))
    (switch-to-buffer buf)
    (erase-buffer)
    (outline-mode)

    (insert "* Chapter 1: Introduction\n")
    (insert "\n")
    (insert "Opening paragraph for the introduction.\n")
    (insert "Sets the stage for the document.\n")
    (insert "\n")
    (insert "** Section 1.1: Background\n")
    (insert "\n")
    (insert "Background information and context.\n")
    (insert "Historical overview of the topic.\n")
    (insert "\n")
    (insert "*** Subsection 1.1.1: Early History\n")
    (insert "\n")
    (insert "The earliest known references date back to antiquity.\n")
    (insert "Scholars debated the origins extensively.\n")
    (insert "\n")
    (insert "**** Detail 1.1.1.1: Primary Sources\n")
    (insert "\n")
    (insert "The primary sources include ancient manuscripts.\n")
    (insert "These were preserved in monastic libraries.\n")
    (insert "\n")
    (insert "**** Detail 1.1.1.2: Secondary Sources\n")
    (insert "\n")
    (insert "Secondary sources emerged during the Renaissance.\n")
    (insert "They provided commentary and analysis.\n")
    (insert "\n")
    (insert "*** Subsection 1.1.2: Modern Era\n")
    (insert "\n")
    (insert "The modern era brought new methodologies.\n")
    (insert "Digital tools revolutionized the field.\n")
    (insert "\n")
    (insert "** Section 1.2: Motivation\n")
    (insert "\n")
    (insert "The motivation for this work stems from gaps in knowledge.\n")
    (insert "We aim to address the following questions.\n")
    (insert "\n")
    (insert "* Chapter 2: Methods\n")
    (insert "\n")
    (insert "Description of the methods used.\n")
    (insert "\n")
    (insert "** Section 2.1: Data Collection\n")
    (insert "\n")
    (insert "Data was collected from multiple sources.\n")
    (insert "Each source was validated independently.\n")
    (insert "\n")
    (insert "** Section 2.2: Analysis\n")
    (insert "\n")
    (insert "Statistical analysis was performed.\n")
    (insert "Results were cross-referenced.\n")
    (insert "\n")
    (insert "*** Subsection 2.2.1: Quantitative\n")
    (insert "\n")
    (insert "Quantitative methods included regression.\n")
    (insert "\n")
    (insert "*** Subsection 2.2.2: Qualitative\n")
    (insert "\n")
    (insert "Qualitative methods included interviews.\n")
    (insert "\n")
    (insert "* Chapter 3: Conclusions\n")
    (insert "\n")
    (insert "Summary of findings and future work.\n")
    (insert "Thank you for reading.\n")

    (display-line-numbers-mode 1)
    (goto-char (point-min))

    ;; Show only top-level headings
    (outline-hide-sublevels 1)

    (message "Test 6: Nested outline. C-c C-o to show levels: 1=chapters, 2=+sections, 3=+subsections.")))

;; ============================================================================
;; Test 7: hs-minor-mode in C-like pseudocode
;; ============================================================================

(defun sdt--test-hideshow-c-mode ()
  "Test `hs-minor-mode' with C-like brace-delimited blocks."
  (let ((buf (sdt--make-buffer "hideshow-c")))
    (switch-to-buffer buf)
    (erase-buffer)
    (c-mode)
    (hs-minor-mode 1)

    (insert "/* C-Mode HideShow Test\n")
    (insert " * Fold brace-delimited blocks.\n")
    (insert " * C-c @ C-c to toggle fold at point.\n")
    (insert " * C-c @ C-M-h to fold all, C-c @ C-M-s to show all.\n")
    (insert " */\n")
    (insert "\n")
    (insert "#include <stdio.h>\n")
    (insert "#include <stdlib.h>\n")
    (insert "\n")
    (insert "typedef struct {\n")
    (insert "    int x;\n")
    (insert "    int y;\n")
    (insert "    char name[64];\n")
    (insert "} Point;\n")
    (insert "\n")
    (insert "int factorial(int n) {\n")
    (insert "    if (n <= 1) {\n")
    (insert "        return 1;\n")
    (insert "    }\n")
    (insert "    return n * factorial(n - 1);\n")
    (insert "}\n")
    (insert "\n")
    (insert "void print_array(int *arr, int len) {\n")
    (insert "    for (int i = 0; i < len; i++) {\n")
    (insert "        if (i > 0) {\n")
    (insert "            printf(\", \");\n")
    (insert "        }\n")
    (insert "        printf(\"%d\", arr[i]);\n")
    (insert "    }\n")
    (insert "    printf(\"\\n\");\n")
    (insert "}\n")
    (insert "\n")
    (insert "int main(int argc, char **argv) {\n")
    (insert "    int numbers[] = {5, 3, 8, 1, 9, 2, 7};\n")
    (insert "    int len = sizeof(numbers) / sizeof(numbers[0]);\n")
    (insert "\n")
    (insert "    printf(\"Original: \");\n")
    (insert "    print_array(numbers, len);\n")
    (insert "\n")
    (insert "    /* Bubble sort */\n")
    (insert "    for (int i = 0; i < len - 1; i++) {\n")
    (insert "        for (int j = 0; j < len - i - 1; j++) {\n")
    (insert "            if (numbers[j] > numbers[j + 1]) {\n")
    (insert "                int temp = numbers[j];\n")
    (insert "                numbers[j] = numbers[j + 1];\n")
    (insert "                numbers[j + 1] = temp;\n")
    (insert "            }\n")
    (insert "        }\n")
    (insert "    }\n")
    (insert "\n")
    (insert "    printf(\"Sorted: \");\n")
    (insert "    print_array(numbers, len);\n")
    (insert "\n")
    (insert "    for (int i = 0; i < len; i++) {\n")
    (insert "        printf(\"%d! = %d\\n\", numbers[i], factorial(numbers[i]));\n")
    (insert "    }\n")
    (insert "\n")
    (insert "    return 0;\n")
    (insert "}\n")

    (font-lock-ensure)
    (display-line-numbers-mode 1)
    (goto-char (point-min))

    ;; Fold all blocks
    (hs-hide-all)

    (message "Test 7: hs-minor-mode C-mode. C-c @ C-c toggle, C-c @ C-M-s show all.")))

;; ============================================================================
;; Test 8: Cursor navigation through folded regions
;; ============================================================================

(defun sdt--test-cursor-navigation ()
  "Test cursor movement (C-n, C-p, C-f, C-b) through folded/invisible regions."
  (let ((buf (sdt--make-buffer "cursor-navigation")))
    (switch-to-buffer buf)
    (erase-buffer)
    (outline-mode)

    (sdt--insert-heading "CURSOR NAVIGATION THROUGH FOLDED REGIONS")
    (insert "Move the cursor with C-n/C-p/C-f/C-b through folded headings.\n")
    (insert "The cursor should skip over invisible text correctly.\n")
    (insert "Watch the line number display as you navigate.\n\n")

    (insert "* Heading A\n")
    (insert "\n")
    (insert "Body line A-1.\n")
    (insert "Body line A-2.\n")
    (insert "Body line A-3.\n")
    (insert "Body line A-4.\n")
    (insert "Body line A-5.\n")
    (insert "\n")
    (insert "* Heading B\n")
    (insert "\n")
    (insert "Body line B-1.\n")
    (insert "Body line B-2.\n")
    (insert "Body line B-3.\n")
    (insert "\n")
    (insert "** Sub-Heading B.1\n")
    (insert "\n")
    (insert "Body line B.1-1.\n")
    (insert "Body line B.1-2.\n")
    (insert "\n")
    (insert "** Sub-Heading B.2\n")
    (insert "\n")
    (insert "Body line B.2-1.\n")
    (insert "Body line B.2-2.\n")
    (insert "Body line B.2-3.\n")
    (insert "\n")
    (insert "* Heading C\n")
    (insert "\n")
    (insert "Body line C-1.\n")
    (insert "Body line C-2.\n")
    (insert "\n")
    (insert "* Heading D\n")
    (insert "\n")
    (insert "Body line D-1.\n")
    (insert "Last line of document.\n")

    (display-line-numbers-mode 1)
    (goto-char (point-min))

    ;; Fold all body text
    (outline-hide-body)

    ;; Provide convenient keybindings for showing levels
    (local-set-key (kbd "C-c 1")
                   (lambda () (interactive)
                     (outline-hide-sublevels 1)
                     (message "Showing only level-1 headings")))
    (local-set-key (kbd "C-c 2")
                   (lambda () (interactive)
                     (outline-hide-sublevels 2)
                     (message "Showing level-1 and level-2 headings")))
    (local-set-key (kbd "C-c a")
                   (lambda () (interactive)
                     (outline-show-all)
                     (message "All text visible")))
    (local-set-key (kbd "C-c h")
                   (lambda () (interactive)
                     (outline-hide-body)
                     (message "Body text hidden")))

    (message "Test 8: Cursor navigation. C-n/C-p through folded headings. C-c 1/2 set level, C-c a/h toggle.")))

;; ============================================================================
;; Test 9: Folding interaction with line numbers
;; ============================================================================

(defun sdt--test-line-numbers-with-folding ()
  "Test that line numbers remain correct and properly rendered when text is folded."
  (let ((buf (sdt--make-buffer "line-numbers-folding")))
    (switch-to-buffer buf)
    (erase-buffer)
    (emacs-lisp-mode)
    (outline-minor-mode 1)
    (setq-local outline-regexp ";;;+ ")

    (sdt--insert-heading "LINE NUMBERS + FOLDING INTERACTION")
    (insert "Verify that line numbers display correctly when sections are folded.\n")
    (insert "When folded, line numbers should jump (e.g., 5 -> 15) across hidden lines.\n")
    (insert "The renderer must handle non-contiguous line number sequences.\n\n")
    (sdt--insert-hint "C-c d   -- cycle display-line-numbers (absolute / relative / visual / off)")
    (sdt--insert-hint "C-c f   -- fold all sections")
    (sdt--insert-hint "C-c s   -- show all sections")
    (insert "\n")

    ;; Generate numbered content in sections
    (insert ";;; Section Alpha (lines 1-10)\n")
    (dotimes (i 10)
      (insert (format "(defvar alpha-%02d %d \"Variable alpha-%02d.\")\n" (1+ i) (1+ i) (1+ i))))
    (insert "\n")

    (insert ";;; Section Beta (lines 11-25)\n")
    (dotimes (i 15)
      (insert (format "(defvar beta-%02d %d \"Variable beta-%02d.\")\n" (1+ i) (* (1+ i) 10) (1+ i))))
    (insert "\n")

    (insert ";;; Section Gamma (lines 26-35)\n")
    (dotimes (i 10)
      (insert (format "(defvar gamma-%02d %d \"Variable gamma-%02d.\")\n" (1+ i) (* (1+ i) 100) (1+ i))))
    (insert "\n")

    (insert ";;; Section Delta (lines 36-40)\n")
    (dotimes (i 5)
      (insert (format "(defvar delta-%02d %d \"Variable delta-%02d.\")\n" (1+ i) (* (1+ i) 1000) (1+ i))))
    (insert "\n")

    (insert ";;; End\n")

    (font-lock-ensure)
    (display-line-numbers-mode 1)
    (goto-char (point-min))

    ;; Keybinding: cycle line number display style
    (let ((styles '(t relative visual nil))
          (idx 0))
      (local-set-key (kbd "C-c d")
                     (lambda () (interactive)
                       (setq idx (mod (1+ idx) 4))
                       (setq display-line-numbers (nth idx styles))
                       (message "display-line-numbers: %s" (nth idx styles)))))
    (local-set-key (kbd "C-c f")
                   (lambda () (interactive)
                     (outline-hide-body)
                     (message "All sections folded")))
    (local-set-key (kbd "C-c s")
                   (lambda () (interactive)
                     (outline-show-all)
                     (message "All sections visible")))

    ;; Start folded
    (outline-hide-body)

    (message "Test 9: Line numbers + folding. C-c d cycle style, C-c f fold, C-c s show.")))

;; ============================================================================
;; Test runner
;; ============================================================================

(defvar sdt--tests
  '(("selective-display levels"       . sdt--test-selective-display)
    ("outline-mode headings"          . sdt--test-outline-mode)
    ("outline-minor-mode Emacs Lisp"  . sdt--test-outline-minor-mode)
    ("hs-minor-mode (hideshow)"       . sdt--test-hideshow)
    ("invisible text property"        . sdt--test-invisible-property)
    ("nested outline folding"         . sdt--test-nested-outline)
    ("hideshow C-mode"                . sdt--test-hideshow-c-mode)
    ("cursor navigation"              . sdt--test-cursor-navigation)
    ("line numbers + folding"         . sdt--test-line-numbers-with-folding))
  "Alist of (name . function) for selective display tests.")

(defun sdt-next ()
  "Run the next test in the sequence."
  (interactive)
  (when (>= sdt--test-step (length sdt--tests))
    (setq sdt--test-step 0))
  (let ((test (nth sdt--test-step sdt--tests)))
    (message "--- Running test %d/%d: %s ---"
             (1+ sdt--test-step)
             (length sdt--tests)
             (car test))
    (funcall (cdr test))
    (setq sdt--test-step (1+ sdt--test-step))))

(defun sdt-run-all ()
  "Run all tests sequentially with pauses."
  (interactive)
  (setq sdt--test-step 0)
  (let ((i 0))
    (dolist (test sdt--tests)
      (run-at-time (* i 3) nil
                   (lambda (test-pair idx)
                     (message "\n=== Test %d/%d: %s ==="
                              (1+ idx)
                              (length sdt--tests)
                              (car test-pair))
                     (funcall (cdr test-pair)))
                   test i)
      (setq i (1+ i)))
    (run-at-time (* i 3) nil
                 (lambda ()
                   (message "\n=== All %d selective display tests complete ===" (length sdt--tests))
                   (message "Press 'q' to clean up, 'n' for next test, 'a' to run all again"))))
  (message "Running all %d tests (3 second intervals)..." (length sdt--tests)))

;; Keybindings for interactive use
(defvar sdt-map (make-sparse-keymap)
  "Keymap for selective display test.")

(define-key sdt-map (kbd "n") #'sdt-next)
(define-key sdt-map (kbd "a") #'sdt-run-all)
(define-key sdt-map (kbd "q") (lambda () (interactive)
                                (sdt--cleanup)
                                (message "All test buffers cleaned up.")))
(define-key sdt-map (kbd "1") (lambda () (interactive) (sdt--test-selective-display)))
(define-key sdt-map (kbd "2") (lambda () (interactive) (sdt--test-outline-mode)))
(define-key sdt-map (kbd "3") (lambda () (interactive) (sdt--test-outline-minor-mode)))
(define-key sdt-map (kbd "4") (lambda () (interactive) (sdt--test-hideshow)))
(define-key sdt-map (kbd "5") (lambda () (interactive) (sdt--test-invisible-property)))
(define-key sdt-map (kbd "6") (lambda () (interactive) (sdt--test-nested-outline)))
(define-key sdt-map (kbd "7") (lambda () (interactive) (sdt--test-hideshow-c-mode)))
(define-key sdt-map (kbd "8") (lambda () (interactive) (sdt--test-cursor-navigation)))
(define-key sdt-map (kbd "9") (lambda () (interactive) (sdt--test-line-numbers-with-folding)))

(set-transient-map sdt-map t)

;; Setup on load
(message "=== Selective Display / Folding Test Suite ===")
(message "Keys: n=next  a=run-all  1-9=specific test  q=cleanup")
(sdt--test-selective-display)

;;; selective-display-test.el ends here
