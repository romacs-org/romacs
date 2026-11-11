;;; whitespace-test.el --- Test whitespace visualization and display -*- lexical-binding: t -*-

;; Test whitespace-mode rendering, tab display, trailing whitespace,
;; special whitespace characters, and related display features in the
;; romacs GPU renderer.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/whitespace-test.el

;;; Code:

(require 'whitespace)

(defvar whitespace-test--step 0
  "Current test step for the automated sequence.")

(defun whitespace-test--insert-heading (title)
  "Insert a section heading with TITLE."
  (insert "\n")
  (let ((start (point)))
    (insert (format "=== %s ===\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold" :height 1.2))))

(defun whitespace-test--insert-label (label)
  "Insert a left-aligned LABEL."
  (let ((start (point)))
    (insert (format "  %-40s" label))
    (put-text-property start (point) 'face '(:foreground "light steel blue"))))

(defun whitespace-test--make-buffer (name)
  "Create and switch to a test buffer named NAME, returning it."
  (let ((buf (get-buffer-create name)))
    (switch-to-buffer buf)
    (erase-buffer)
    buf))

;; ============================================================================
;; Test 1: whitespace-mode with various whitespace-style settings
;; ============================================================================

(defun whitespace-test--styles ()
  "Test whitespace-mode with different whitespace-style configurations."
  (whitespace-test--make-buffer "*WS Test: Styles*")
  (let ((start (point)))
    (insert "WHITESPACE-MODE STYLE SETTINGS\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n")

  ;; Spaces visualization
  (whitespace-test--insert-heading "whitespace-style: (spaces)")
  (insert "  Spaces between words should be visualized with dots/marks:\n")
  (insert "  hello   world     test      end\n")
  (insert "  one two three four five six seven\n")

  ;; Tabs visualization
  (whitespace-test--insert-heading "whitespace-style: (tabs)")
  (insert "  Tab characters should be visualized with arrows/marks:\n")
  (insert "	single tab\n")
  (insert "		double tab\n")
  (insert "			triple tab\n")
  (insert "	col1	col2	col3	col4\n")

  ;; Trailing whitespace
  (whitespace-test--insert-heading "whitespace-style: (trailing)")
  (insert "  Lines below have trailing whitespace (should be highlighted):\n")
  (insert "  trailing spaces   \n")
  (insert "  trailing tabs		\n")
  (insert "  trailing mixed 	 	\n")
  (insert "  no trailing here\n")

  ;; Lines (long lines)
  (whitespace-test--insert-heading "whitespace-style: (lines)")
  (insert "  Lines exceeding whitespace-line-column should be highlighted:\n")
  (insert "  short line\n")
  (insert "  this is a medium length line that does not exceed the default column limit of eighty characters\n")
  (insert "  this is a very long line that should definitely exceed the whitespace-line-column threshold and be highlighted by whitespace-mode when the lines style is active in the configuration\n")

  ;; Empty lines
  (whitespace-test--insert-heading "whitespace-style: (empty)")
  (insert "  Empty lines at buffer beginning/end should be marked.\n")
  (insert "  (See Test 8 for a dedicated empty-lines buffer.)\n")

  ;; Combined styles
  (whitespace-test--insert-heading "whitespace-style: (spaces tabs trailing newline-mark)")
  (insert "  All whitespace types visualized together:\n")
  (insert "	tab	then spaces   then trailing   \n")
  (insert "  spaces only between words\n")
  (insert "		tabs		and more tabs\n")
  (insert "  mixed	content	here   \n")

  ;; Enable whitespace-mode with full style
  (setq-local whitespace-style
              '(face spaces tabs trailing lines empty
                space-mark tab-mark newline-mark))
  (setq-local whitespace-line-column 80)
  (whitespace-mode 1)
  (goto-char (point-min))
  (message "Test 1: whitespace-mode styles (spaces, tabs, trailing, lines, empty, marks)"))

;; ============================================================================
;; Test 2: Tab rendering at different tab-width values
;; ============================================================================

(defun whitespace-test--tab-widths ()
  "Test tab rendering at tab-width 2, 4, and 8."
  (whitespace-test--make-buffer "*WS Test: Tab Widths*")
  (let ((start (point)))
    (insert "TAB RENDERING AT DIFFERENT TAB-WIDTH VALUES\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n")

  (whitespace-test--insert-heading "Reference: pipe-delimited columns")
  (insert "  |0       |8       |16      |24      |32      |40\n")

  ;; Tab-width 2
  (whitespace-test--insert-heading "tab-width = 2")
  (insert "  Tabs should advance 2 columns:\n")
  (insert "	a\n")
  (insert "		b\n")
  (insert "			c\n")
  (insert "				d\n")
  (insert "	1	2	3	4	5	6	7	8\n")

  ;; Tab-width 4
  (whitespace-test--insert-heading "tab-width = 4")
  (insert "  Tabs should advance 4 columns:\n")
  (insert "	a\n")
  (insert "		b\n")
  (insert "			c\n")
  (insert "	1	2	3	4	5\n")

  ;; Tab-width 8
  (whitespace-test--insert-heading "tab-width = 8")
  (insert "  Tabs should advance 8 columns (default):\n")
  (insert "	a\n")
  (insert "		b\n")
  (insert "			c\n")
  (insert "	1	2	3	4\n")

  ;; Create three windows showing different tab-widths
  (delete-other-windows)
  (let ((buf2 (get-buffer-create "*WS Tab-Width 2*"))
        (buf4 (get-buffer-create "*WS Tab-Width 4*"))
        (buf8 (get-buffer-create "*WS Tab-Width 8*")))
    ;; Populate each buffer with identical tab content
    (dolist (pair `((,buf2 . 2) (,buf4 . 4) (,buf8 . 8)))
      (with-current-buffer (car pair)
        (erase-buffer)
        (let ((tw (cdr pair))
              (start (point)))
          (insert (format "tab-width = %d\n" tw))
          (put-text-property start (point) 'face '(:weight bold :foreground "gold")))
        (insert (make-string 40 ?-) "\n")
        (insert "	level 1\n")
        (insert "		level 2\n")
        (insert "			level 3\n")
        (insert "				level 4\n")
        (insert "	col-a	col-b	col-c	col-d\n")
        (insert "	1	22	333	4444\n")
        (insert "	hello	world	foo	bar\n")
        (setq-local tab-width (cdr pair))
        (setq-local whitespace-style '(face tabs tab-mark))
        (whitespace-mode 1)
        (goto-char (point-min))))
    ;; Split into three windows
    (switch-to-buffer buf2)
    (split-window-right)
    (other-window 1)
    (switch-to-buffer buf4)
    (split-window-right)
    (other-window 1)
    (switch-to-buffer buf8)
    (other-window -2))
  (message "Test 2: Tab widths — 3 windows showing tab-width 2, 4, 8"))

;; ============================================================================
;; Test 3: Trailing whitespace highlighting (show-trailing-whitespace)
;; ============================================================================

(defun whitespace-test--trailing ()
  "Test show-trailing-whitespace and whitespace-mode trailing detection."
  (whitespace-test--make-buffer "*WS Test: Trailing*")
  (let ((start (point)))
    (insert "TRAILING WHITESPACE HIGHLIGHTING\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n\n")

  (whitespace-test--insert-heading "show-trailing-whitespace = t")
  (insert "  Lines with trailing spaces (should show red/pink background):\n")
  (insert "  three trailing spaces   \n")
  (insert "  five trailing spaces     \n")
  (insert "  trailing tab	\n")
  (insert "  trailing tab+spaces	   \n")
  (insert "  trailing spaces+tab   	\n")
  (insert "  no trailing\n")
  (insert "  only a newline follows\n")

  (whitespace-test--insert-heading "Blank lines (all whitespace)")
  (insert "  The next 3 lines contain only spaces/tabs:\n")
  (insert "     \n")
  (insert "		\n")
  (insert " 	 	 \n")
  (insert "  End of blank lines section.\n")

  (whitespace-test--insert-heading "Cursor position interaction")
  (insert "  When point is at EOL, trailing whitespace on THAT line\n")
  (insert "  may not be highlighted (Emacs default behavior).\n")
  (insert "  Move cursor to different lines and observe:   \n")
  (insert "  another line with trailing   \n")
  (insert "  and one more   \n")

  (setq-local show-trailing-whitespace t)
  ;; Also enable whitespace-mode trailing for double coverage
  (setq-local whitespace-style '(face trailing))
  (whitespace-mode 1)
  (goto-char (point-min))
  (message "Test 3: Trailing whitespace — show-trailing-whitespace + whitespace-mode"))

;; ============================================================================
;; Test 4: Mixed tabs and spaces visualization
;; ============================================================================

(defun whitespace-test--mixed ()
  "Test visualization of mixed tabs and spaces."
  (whitespace-test--make-buffer "*WS Test: Mixed*")
  (let ((start (point)))
    (insert "MIXED TABS AND SPACES VISUALIZATION\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n\n")

  (whitespace-test--insert-heading "Indentation: tabs only")
  (insert "	one tab\n")
  (insert "		two tabs\n")
  (insert "			three tabs\n")

  (whitespace-test--insert-heading "Indentation: spaces only (4-space)")
  (insert "    four spaces\n")
  (insert "        eight spaces\n")
  (insert "            twelve spaces\n")

  (whitespace-test--insert-heading "Indentation: mixed (tab then spaces)")
  (insert "	 tab + 1 space\n")
  (insert "	  tab + 2 spaces\n")
  (insert "	   tab + 3 spaces\n")
  (insert "		 2 tabs + 1 space\n")

  (whitespace-test--insert-heading "Indentation: mixed (spaces then tab)")
  (insert "   	3 spaces then tab\n")
  (insert "  	2 spaces then tab\n")
  (insert " 	1 space then tab\n")

  (whitespace-test--insert-heading "Indentation: alternating")
  (insert " 	 	 space-tab-space-tab-space\n")
  (insert "	 	 tab-space-tab-space\n")
  (insert "  	  	  2sp-tab-2sp-tab-2sp\n")

  (whitespace-test--insert-heading "Alignment after content")
  (insert "short	tab-aligned\n")
  (insert "medium text	tab-aligned\n")
  (insert "a longer piece of text	tab-aligned\n")

  (setq-local tab-width 4)
  (setq-local whitespace-style
              '(face indentation spaces tabs space-mark tab-mark
                space-before-tab space-after-tab))
  (whitespace-mode 1)
  (goto-char (point-min))
  (message "Test 4: Mixed tabs and spaces — indentation patterns and alignment"))

;; ============================================================================
;; Test 5: Non-breaking spaces and special whitespace characters
;; ============================================================================

(defun whitespace-test--special-chars ()
  "Test rendering of non-breaking spaces, zero-width chars, and other specials."
  (whitespace-test--make-buffer "*WS Test: Special Chars*")
  (let ((start (point)))
    (insert "NON-BREAKING SPACES AND SPECIAL WHITESPACE\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n\n")

  (whitespace-test--insert-heading "Non-breaking space (U+00A0)")
  (whitespace-test--insert-label "between words:")
  (insert "hello\u00A0world\n")
  (whitespace-test--insert-label "multiple:")
  (insert "a\u00A0\u00A0\u00A0b\u00A0\u00A0\u00A0c\n")
  (whitespace-test--insert-label "mixed with regular space:")
  (insert "regular \u00A0nbsp\u00A0 regular \u00A0nbsp\n")

  (whitespace-test--insert-heading "Narrow no-break space (U+202F)")
  (whitespace-test--insert-label "between words:")
  (insert "hello\u202Fworld\n")
  (whitespace-test--insert-label "multiple:")
  (insert "a\u202F\u202F\u202Fb\n")

  (whitespace-test--insert-heading "Em space (U+2003)")
  (whitespace-test--insert-label "between words:")
  (insert "hello\u2003world\n")
  (whitespace-test--insert-label "multiple:")
  (insert "a\u2003\u2003\u2003b\n")

  (whitespace-test--insert-heading "En space (U+2002)")
  (whitespace-test--insert-label "between words:")
  (insert "hello\u2002world\n")
  (whitespace-test--insert-label "multiple:")
  (insert "a\u2002\u2002\u2002b\n")

  (whitespace-test--insert-heading "Thin space (U+2009)")
  (whitespace-test--insert-label "between words:")
  (insert "hello\u2009world\n")
  (whitespace-test--insert-label "multiple:")
  (insert "a\u2009\u2009\u2009b\n")

  (whitespace-test--insert-heading "Ideographic space (U+3000)")
  (whitespace-test--insert-label "between words:")
  (insert "hello\u3000world\n")
  (whitespace-test--insert-label "multiple:")
  (insert "a\u3000\u3000\u3000b\n")

  (whitespace-test--insert-heading "Zero-width space (U+200B)")
  (whitespace-test--insert-label "between words (invisible):")
  (insert "hello\u200Bworld\n")
  (whitespace-test--insert-label "should look identical:")
  (insert "helloworld\n")

  (whitespace-test--insert-heading "Comparison: all space types on one line")
  (insert "  REG")
  (insert " ")
  (insert "NBSP")
  (insert "\u00A0")
  (insert "NNBSP")
  (insert "\u202F")
  (insert "EM")
  (insert "\u2003")
  (insert "EN")
  (insert "\u2002")
  (insert "THIN")
  (insert "\u2009")
  (insert "IDEO")
  (insert "\u3000")
  (insert "END\n")

  (setq-local whitespace-style
              '(face spaces tabs space-mark tab-mark newline-mark))
  ;; Configure whitespace-display-mappings to show special chars
  (setq-local whitespace-display-mappings
              '((space-mark ?\u00A0 [?\u00B7] [?_])   ; NBSP -> middle dot
                (space-mark ?\u3000 [?\u25A1] [?_])    ; ideographic -> white square
                (space-mark ?\  [?\u00B7] [?.])        ; regular space -> middle dot
                (tab-mark ?\t [?\u2192 ?\t] [?> ?\t])  ; tab -> right arrow
                (newline-mark ?\n [?\u21B5 ?\n] [?$ ?\n]))) ; newline -> return symbol
  (whitespace-mode 1)
  (goto-char (point-min))
  (message "Test 5: Special whitespace characters (NBSP, em/en/thin/ideo space, ZWSP)"))

;; ============================================================================
;; Test 6: whitespace-display-mappings customization
;; ============================================================================

(defun whitespace-test--display-mappings ()
  "Test custom whitespace-display-mappings rendering."
  (whitespace-test--make-buffer "*WS Test: Display Mappings*")
  (let ((start (point)))
    (insert "WHITESPACE DISPLAY MAPPINGS CUSTOMIZATION\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n\n")

  (whitespace-test--insert-heading "Default mappings")
  (insert "  Spaces, tabs, and newlines with default display-mappings:\n")
  (insert "  hello   world	tab	here\n")
  (insert "  indented    with     spaces\n")
  (insert "	tab	indented	line\n\n")

  (whitespace-test--insert-heading "Custom mapping: space -> middle dot (U+00B7)")
  (insert "  Each space should appear as a centered dot:\n")
  (insert "  one two three four five\n")
  (insert "      indented with spaces\n\n")

  (whitespace-test--insert-heading "Custom mapping: tab -> right arrow (U+2192)")
  (insert "  Each tab should show a right-pointing arrow:\n")
  (insert "	single\n")
  (insert "		double\n")
  (insert "	col1	col2	col3\n\n")

  (whitespace-test--insert-heading "Custom mapping: newline -> return symbol (U+21B5)")
  (insert "  Each line ending should show a return/enter symbol:\n")
  (insert "  first line\n")
  (insert "  second line\n")
  (insert "  third line\n\n")

  (whitespace-test--insert-heading "All custom mappings together")
  (insert "  Spaces=dot, tabs=arrow, newlines=return:\n")
  (insert "  hello 	world	 	end\n")
  (insert "	 indented	with	all types \n")
  (insert "  final   line   with   spaces\n")

  ;; Apply custom display mappings
  (setq-local whitespace-display-mappings
              '((space-mark ?\  [?\u00B7] [?.])         ; space -> middle dot
                (space-mark ?\u00A0 [?\u00A4] [?_])     ; NBSP -> currency sign
                (tab-mark ?\t [?\u2192 ?\t] [?> ?\t])   ; tab -> right arrow + tab
                (newline-mark ?\n [?\u21B5 ?\n] [?$ ?\n]))) ; newline -> return symbol
  (setq-local whitespace-style
              '(face spaces tabs newline
                space-mark tab-mark newline-mark))
  (whitespace-mode 1)
  (goto-char (point-min))
  (message "Test 6: Custom display mappings (dot=space, arrow=tab, return=newline)"))

;; ============================================================================
;; Test 7: Long lines with whitespace-line-column
;; ============================================================================

(defun whitespace-test--long-lines ()
  "Test long-line highlighting with different whitespace-line-column values."
  (whitespace-test--make-buffer "*WS Test: Long Lines*")
  (let ((start (point)))
    (insert "LONG LINE HIGHLIGHTING (whitespace-line-column)\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n\n")

  ;; Column ruler
  (whitespace-test--insert-heading "Column ruler")
  (insert "  ")
  (let ((start (point)))
    (insert "0         1         2         3         4         5         6         7         8         9         10        11        12\n")
    (put-text-property start (point) 'face '(:foreground "dark gray")))
  (insert "  ")
  (let ((start (point)))
    (insert "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890\n")
    (put-text-property start (point) 'face '(:foreground "dark gray")))

  (whitespace-test--insert-heading "whitespace-line-column = 80 (default)")
  (insert "  Short line (under 80).\n")
  (insert "  This line is exactly eighty characters long, padding it out to reach that goal.\n")
  (insert "  This line deliberately exceeds eighty characters and should be highlighted when the lines style is active in whitespace-mode configuration.\n")
  (insert "  Another very long line with lots of text that goes well past the column limit to test that the entire tail portion after column 80 gets highlighted.\n")

  (whitespace-test--insert-heading "Lines with trailing whitespace + long")
  (insert "  This is a long line that exceeds eighty characters and also has trailing whitespace at the very end of it to test both features   \n")
  (insert "  Short with trailing   \n")

  (whitespace-test--insert-heading "Lines at exactly the boundary")
  ;; Generate lines of specific lengths
  (dolist (len '(78 79 80 81 82))
    (whitespace-test--insert-label (format "length = %d:" len))
    (insert (make-string (max 0 (- len 2)) ?x))
    (insert "\n"))

  (whitespace-test--insert-heading "whitespace-line-column = 40 (narrow)")
  (insert "  With column set to 40:\n")
  (insert "  Short.\n")
  (insert "  This is over forty characters long, exceeds.\n")
  (insert "  Even more text to show highlighting at col 40.\n")

  (setq-local whitespace-line-column 80)
  (setq-local whitespace-style '(face lines trailing))
  (whitespace-mode 1)
  (goto-char (point-min))
  (message "Test 7: Long lines — whitespace-line-column = 80, lines style"))

;; ============================================================================
;; Test 8: Empty lines visualization
;; ============================================================================

(defun whitespace-test--empty-lines ()
  "Test empty-line visualization at buffer start and end."
  (whitespace-test--make-buffer "*WS Test: Empty Lines*")

  ;; Leading empty lines
  (insert "\n")
  (insert "\n")
  (insert "\n")

  (let ((start (point)))
    (insert "EMPTY LINE VISUALIZATION\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n\n")
  (insert "There are 3 empty lines ABOVE this text (buffer start).\n")
  (insert "There are 3 empty lines BELOW this text (buffer end).\n")
  (insert "whitespace-mode with 'empty' style should mark them.\n\n")

  (whitespace-test--insert-heading "Middle empty lines")
  (insert "  Text before empty lines.\n")
  (insert "\n")
  (insert "\n")
  (insert "\n")
  (insert "  Text after 3 empty lines.\n")
  (insert "  (Middle empty lines are NOT highlighted by 'empty' style;\n")
  (insert "   only buffer-start and buffer-end empty lines are.)\n")

  (whitespace-test--insert-heading "Lines with only whitespace")
  (insert "  Next 3 lines have spaces/tabs but no visible content:\n")
  (insert "     \n")
  (insert "	\n")
  (insert " 	 \n")
  (insert "  End of whitespace-only lines.\n")

  ;; Trailing empty lines
  (insert "\n")
  (insert "\n")
  (insert "\n")

  (setq-local whitespace-style '(face empty trailing))
  (whitespace-mode 1)
  (goto-char (point-min))
  (message "Test 8: Empty lines — buffer start/end empty line highlighting"))

;; ============================================================================
;; Test 9: Indentation visualization (tabs vs spaces)
;; ============================================================================

(defun whitespace-test--indentation ()
  "Test indentation visualization: indent-tabs-mode interaction."
  (whitespace-test--make-buffer "*WS Test: Indentation*")
  (let ((start (point)))
    (insert "INDENTATION VISUALIZATION\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n\n")

  (whitespace-test--insert-heading "indent-tabs-mode = t (prefer tabs)")
  (insert "  When indent-tabs-mode is t, space indentation is flagged:\n")
  (insert "	tab-indented (correct)\n")
  (insert "		double-tab (correct)\n")
  (insert "    space-indented (flagged)\n")
  (insert "        8-space indent (flagged)\n")
  (insert "	 tab+space (space after tab flagged)\n")

  (whitespace-test--insert-heading "indent-tabs-mode = nil (prefer spaces)")
  (insert "  When indent-tabs-mode is nil, tab indentation is flagged:\n")
  (insert "    space-indented (correct)\n")
  (insert "        double-space-indent (correct)\n")
  (insert "	tab-indented (flagged)\n")
  (insert "		double-tab (flagged)\n")
  (insert "   	space-then-tab (flagged)\n")

  (whitespace-test--insert-heading "Consistent indentation patterns")
  (insert "  Code-like indentation (4 spaces):\n")
  (insert "    if (condition) {\n")
  (insert "        body;\n")
  (insert "            nested;\n")
  (insert "        end_body;\n")
  (insert "    }\n\n")

  (insert "  Code-like indentation (tabs):\n")
  (insert "	if (condition) {\n")
  (insert "		body;\n")
  (insert "			nested;\n")
  (insert "		end_body;\n")
  (insert "	}\n\n")

  (insert "  Lisp-style indentation (2 spaces):\n")
  (insert "  (defun foo (x)\n")
  (insert "    (let ((y (+ x 1)))\n")
  (insert "      (when (> y 0)\n")
  (insert "        (message \"%d\" y))))\n")

  (whitespace-test--insert-heading "space-before-tab / space-after-tab")
  (insert "  These patterns highlight spaces adjacent to tabs:\n")
  (insert "   	space-before-tab\n")
  (insert "	   space-after-tab\n")
  (insert "  	 space-before-AND-after-tab\n")
  (insert "	tab-only (no flag)\n")
  (insert "    spaces-only (no flag when indent-tabs-mode=nil)\n")

  (setq-local indent-tabs-mode t)
  (setq-local tab-width 4)
  (setq-local whitespace-style
              '(face indentation tabs spaces
                space-before-tab space-after-tab
                tab-mark space-mark))
  (whitespace-mode 1)
  (goto-char (point-min))
  (message "Test 9: Indentation — tabs vs spaces, space-before/after-tab"))

;; ============================================================================
;; Test 10: Interaction with indent-tabs-mode
;; ============================================================================

(defun whitespace-test--indent-tabs-interaction ()
  "Test whitespace-mode behavior with different indent-tabs-mode settings."
  (delete-other-windows)

  ;; Buffer with indent-tabs-mode = t
  (let ((buf-tabs (get-buffer-create "*WS indent-tabs=t*"))
        (buf-spaces (get-buffer-create "*WS indent-tabs=nil*")))

    ;; Tabs buffer
    (with-current-buffer buf-tabs
      (erase-buffer)
      (let ((start (point)))
        (insert "indent-tabs-mode = t\n")
        (put-text-property start (point) 'face '(:weight bold :foreground "lime green" :height 1.2)))
      (insert (make-string 40 ?-) "\n\n")
      (insert "Correct (tab) indentation:\n")
      (insert "	level 1\n")
      (insert "		level 2\n")
      (insert "			level 3\n\n")
      (insert "Wrong (space) indentation:\n")
      (insert "    level 1\n")
      (insert "        level 2\n")
      (insert "            level 3\n\n")
      (insert "Mixed (should flag):\n")
      (insert "	 tab+space\n")
      (insert "   	space+tab\n")
      (insert " 	 	alt-space-tab\n")
      (setq-local indent-tabs-mode t)
      (setq-local tab-width 4)
      (setq-local whitespace-style
                  '(face indentation tabs spaces trailing
                    space-before-tab space-after-tab
                    tab-mark space-mark))
      (whitespace-mode 1)
      (goto-char (point-min)))

    ;; Spaces buffer
    (with-current-buffer buf-spaces
      (erase-buffer)
      (let ((start (point)))
        (insert "indent-tabs-mode = nil\n")
        (put-text-property start (point) 'face '(:weight bold :foreground "deep sky blue" :height 1.2)))
      (insert (make-string 40 ?-) "\n\n")
      (insert "Correct (space) indentation:\n")
      (insert "    level 1\n")
      (insert "        level 2\n")
      (insert "            level 3\n\n")
      (insert "Wrong (tab) indentation:\n")
      (insert "	level 1\n")
      (insert "		level 2\n")
      (insert "			level 3\n\n")
      (insert "Mixed (should flag):\n")
      (insert "	 tab+space\n")
      (insert "   	space+tab\n")
      (insert " 	 	alt-space-tab\n")
      (setq-local indent-tabs-mode nil)
      (setq-local tab-width 4)
      (setq-local whitespace-style
                  '(face indentation tabs spaces trailing
                    space-before-tab space-after-tab
                    tab-mark space-mark))
      (whitespace-mode 1)
      (goto-char (point-min)))

    ;; Show side-by-side
    (switch-to-buffer buf-tabs)
    (split-window-right)
    (other-window 1)
    (switch-to-buffer buf-spaces)
    (other-window -1))

  (message "Test 10: indent-tabs-mode — left=tabs-preferred, right=spaces-preferred"))

;; ============================================================================
;; Test runner
;; ============================================================================

(defvar whitespace-test--tests
  '(("Whitespace styles"         . whitespace-test--styles)
    ("Tab widths (2, 4, 8)"      . whitespace-test--tab-widths)
    ("Trailing whitespace"       . whitespace-test--trailing)
    ("Mixed tabs and spaces"     . whitespace-test--mixed)
    ("Special whitespace chars"  . whitespace-test--special-chars)
    ("Display mappings"          . whitespace-test--display-mappings)
    ("Long lines"                . whitespace-test--long-lines)
    ("Empty lines"               . whitespace-test--empty-lines)
    ("Indentation"               . whitespace-test--indentation)
    ("indent-tabs-mode"          . whitespace-test--indent-tabs-interaction))
  "Alist of (name . function) for whitespace tests.")

(defun whitespace-test-next ()
  "Run the next test in the sequence."
  (interactive)
  (when (>= whitespace-test--step (length whitespace-test--tests))
    (setq whitespace-test--step 0))
  (let ((test (nth whitespace-test--step whitespace-test--tests)))
    (delete-other-windows)
    (message "--- Running test %d/%d: %s ---"
             (1+ whitespace-test--step)
             (length whitespace-test--tests)
             (car test))
    (funcall (cdr test))
    (setq whitespace-test--step (1+ whitespace-test--step))))

(defun whitespace-test-run-all ()
  "Run all tests sequentially with pauses."
  (interactive)
  (setq whitespace-test--step 0)
  (let ((i 0))
    (dolist (test whitespace-test--tests)
      (run-at-time (* i 3) nil
                   (lambda (test-pair idx)
                     (delete-other-windows)
                     (message "\n=== Test %d/%d: %s ==="
                              (1+ idx)
                              (length whitespace-test--tests)
                              (car test-pair))
                     (funcall (cdr test-pair)))
                   test i)
      (setq i (1+ i)))
    (run-at-time (* i 3) nil
                 (lambda ()
                   (message "\n=== All %d whitespace tests complete ==="
                            (length whitespace-test--tests))
                   (message "Press 'n' for next, 'a' to run all, 'q' to cleanup, 1-0 for specific"))))
  (message "Running all %d tests (3 second intervals)..." (length whitespace-test--tests)))

(defun whitespace-test--cleanup ()
  "Kill all test buffers."
  (dolist (buf (buffer-list))
    (when (string-match-p "\\*WS \\|\\*WS Test:" (buffer-name buf))
      (kill-buffer buf)))
  (message "All whitespace test buffers cleaned up."))

;; Keybindings for interactive use
(defvar whitespace-test-map (make-sparse-keymap)
  "Keymap for whitespace test.")

(define-key whitespace-test-map (kbd "n") #'whitespace-test-next)
(define-key whitespace-test-map (kbd "a") #'whitespace-test-run-all)
(define-key whitespace-test-map (kbd "q") (lambda () (interactive) (whitespace-test--cleanup)))
(define-key whitespace-test-map (kbd "1") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--styles)))
(define-key whitespace-test-map (kbd "2") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--tab-widths)))
(define-key whitespace-test-map (kbd "3") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--trailing)))
(define-key whitespace-test-map (kbd "4") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--mixed)))
(define-key whitespace-test-map (kbd "5") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--special-chars)))
(define-key whitespace-test-map (kbd "6") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--display-mappings)))
(define-key whitespace-test-map (kbd "7") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--long-lines)))
(define-key whitespace-test-map (kbd "8") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--empty-lines)))
(define-key whitespace-test-map (kbd "9") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--indentation)))
(define-key whitespace-test-map (kbd "0") (lambda () (interactive)
                                            (delete-other-windows)
                                            (whitespace-test--indent-tabs-interaction)))

(set-transient-map whitespace-test-map t)

;; Setup on load
(message "=== Whitespace Visualization Test Suite ===")
(message "Keys: n=next  a=run-all  1-0=specific test  q=cleanup")
(whitespace-test--styles)

;;; whitespace-test.el ends here
