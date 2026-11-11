;;; overlay-test.el --- Test overlay rendering -*- lexical-binding: t -*-

;; Test overlay rendering in the romacs GPU renderer: faces, strings,
;; display properties, invisibility, priorities, and interactive toggling.
;; Usage: ./target/release/romacs -Q -l test/romacs/overlay-test.el

;;; Code:

(defvar overlay-test--buffer nil
  "Buffer used for overlay tests.")

(defvar overlay-test--overlays nil
  "List of overlays created during tests (for cleanup).")

(defvar overlay-test--step 0
  "Current test step for the automated sequence.")

(defun overlay-test--cleanup ()
  "Remove all test overlays and kill test buffer."
  (dolist (ov overlay-test--overlays)
    (when (overlay-buffer ov)
      (delete-overlay ov)))
  (setq overlay-test--overlays nil))

(defun overlay-test--make-ov (beg end &rest props)
  "Create overlay from BEG to END with PROPS, track it, and return it."
  (let ((ov (make-overlay beg end)))
    (while props
      (overlay-put ov (pop props) (pop props)))
    (push ov overlay-test--overlays)
    ov))

(defun overlay-test--setup-buffer ()
  "Create and switch to a fresh test buffer with sample text."
  (setq overlay-test--buffer (get-buffer-create "*Overlay Test*"))
  (switch-to-buffer overlay-test--buffer)
  (setq buffer-read-only nil)
  (erase-buffer)
  overlay-test--buffer)

(defun overlay-test--insert-heading (title)
  "Insert a visually distinct section TITLE."
  (insert "\n")
  (let ((start (point)))
    (insert (format "=== %s ===\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold")))
  (insert "\n"))

;; ============================================================================
;; Test 1: Overlay with `face' property
;; ============================================================================

(defun overlay-test--face ()
  "Test overlay face property: background, foreground, bold, italic."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 1: Overlay Face Property")

  (insert "Normal text before the overlay region.\n")
  (let ((start (point)))
    (insert "This text has a yellow background via overlay face.\n")
    (overlay-test--make-ov start (point)
                           'face '(:background "dark goldenrod" :foreground "white")))

  (let ((start (point)))
    (insert "This text is bold and red via overlay face.\n")
    (overlay-test--make-ov start (point)
                           'face '(:foreground "red" :weight bold)))

  (let ((start (point)))
    (insert "This text is italic with a blue background.\n")
    (overlay-test--make-ov start (point)
                           'face '(:background "dark blue" :foreground "white" :slant italic)))

  (let ((start (point)))
    (insert "This text combines bold, italic, underline, and green foreground.\n")
    (overlay-test--make-ov start (point)
                           'face '(:foreground "green" :weight bold :slant italic :underline t)))

  (insert "Normal text after the overlay region.\n")
  (goto-char (point-min))
  (message "Test 1: Overlay face — background, foreground, bold, italic"))

;; ============================================================================
;; Test 2: Overlay with `before-string'
;; ============================================================================

(defun overlay-test--before-string ()
  "Test overlay before-string property."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 2: Overlay before-string")

  (insert "Line A: beginning of text.\n")
  (let ((start (point)))
    (insert "Line B: this line has a before-string.\n")
    (overlay-test--make-ov start start
                           'before-string ">>> INSERTED BEFORE >>> "))

  (insert "Line C: another line.\n")
  (let ((start (point)))
    (insert "Line D: a second before-string example.\n")
    (overlay-test--make-ov start start
                           'before-string (propertize "[MARKER] "
                                                      'face '(:foreground "cyan" :weight bold))))

  (insert "Line E: end of text.\n")
  (goto-char (point-min))
  (message "Test 2: Overlay before-string — text inserted before overlay region"))

;; ============================================================================
;; Test 3: Overlay with `after-string'
;; ============================================================================

(defun overlay-test--after-string ()
  "Test overlay after-string property."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 3: Overlay after-string")

  (insert "Line A: beginning of text.\n")
  (let ((end (point)))
    (overlay-test--make-ov (1- end) end
                           'after-string " <<< APPENDED AFTER"))

  (insert "Line B: another line.\n")
  (let ((end (point)))
    (overlay-test--make-ov (1- end) end
                           'after-string (propertize " [DONE]"
                                                     'face '(:foreground "lime green" :weight bold))))

  (insert "Line C: last line, no overlay.\n")
  (goto-char (point-min))
  (message "Test 3: Overlay after-string — text appended after overlay region"))

;; ============================================================================
;; Test 4: Overlay with `display' property
;; ============================================================================

(defun overlay-test--display ()
  "Test overlay display property to replace text."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 4: Overlay display Property")

  (insert "Normal text before.\n")
  (let ((start (point)))
    (insert "THIS TEXT IS REPLACED by the display property.\n")
    (overlay-test--make-ov start (point)
                           'display (propertize "[REPLACEMENT TEXT]\n"
                                                'face '(:foreground "magenta" :weight bold))))

  (insert "Normal text between.\n")
  (let ((start (point)))
    (insert "Another replaced region.\n")
    (overlay-test--make-ov start (point)
                           'display (propertize "[display: custom image placeholder]\n"
                                                'face '(:background "dark cyan" :foreground "white"))))

  (insert "Normal text after.\n")
  (goto-char (point-min))
  (message "Test 4: Overlay display — text replaced via display property"))

;; ============================================================================
;; Test 5: Overlay with `invisible' property
;; ============================================================================

(defun overlay-test--invisible ()
  "Test overlay invisible property to hide text."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 5: Overlay invisible Property")

  ;; Ensure our invisible spec is active
  (add-to-invisibility-spec '(overlay-test . t))

  (insert "Visible line one.\n")
  (let ((start (point)))
    (insert "THIS LINE IS HIDDEN by overlay invisible property.\n")
    (overlay-test--make-ov start (point)
                           'invisible 'overlay-test))

  (insert "Visible line two (the line above is invisible).\n")

  (let ((start (point)))
    (insert "HIDDEN WITH ELLIPSIS")
    (overlay-test--make-ov start (point)
                           'invisible 'overlay-test))
  (insert " <-- there should be an ellipsis before this.\n")

  (insert "Visible line three.\n")
  (goto-char (point-min))
  (message "Test 5: Overlay invisible — hidden text with ellipsis"))

;; ============================================================================
;; Test 6: Multiple overlapping overlays with different faces
;; ============================================================================

(defun overlay-test--overlapping ()
  "Test multiple overlapping overlays."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 6: Overlapping Overlays")

  (insert "Overlapping overlay region follows:\n")
  (let ((base (point)))
    (insert "AAAAABBBBBCCCCCDDDDDEEEEE\n")
    ;; Overlay 1: covers AAAAA..CCCCC (chars 0-14) — red bg
    (overlay-test--make-ov base (+ base 15)
                           'face '(:background "dark red" :foreground "white"))
    ;; Overlay 2: covers BBBBB..DDDDD (chars 5-19) — blue bg
    (overlay-test--make-ov (+ base 5) (+ base 20)
                           'face '(:background "dark blue" :foreground "white"))
    ;; The overlap region BBBBB..CCCCC (chars 5-14) gets both faces.
    ;; The last overlay created takes precedence (by default, same priority).
    (insert "  ^ Red(A), Blue>Red overlap(B-C), Blue(D), Normal(E)\n"))

  (insert "\nAnother overlap with face merging:\n")
  (let ((base (point)))
    (insert "Some overlapping styled text here.\n")
    ;; Bold overlay across the whole line
    (overlay-test--make-ov base (+ base 34)
                           'face '(:weight bold))
    ;; Color overlay on "overlapping styled"
    (overlay-test--make-ov (+ base 5) (+ base 24)
                           'face '(:foreground "orange")))

  (goto-char (point-min))
  (message "Test 6: Overlapping overlays — face merging and precedence"))

;; ============================================================================
;; Test 7: Overlay with `priority' property
;; ============================================================================

(defun overlay-test--priority ()
  "Test overlay priority — higher priority overlay wins."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 7: Overlay Priority")

  (insert "Priority test region:\n")
  (let ((base (point)))
    (insert "This text has two overlays with different priorities.\n")
    ;; Low priority: green background
    (overlay-test--make-ov base (point)
                           'face '(:background "dark green" :foreground "white")
                           'priority 10)
    ;; High priority: red background (should win)
    (overlay-test--make-ov base (point)
                           'face '(:background "dark red" :foreground "yellow")
                           'priority 100))
  (insert "  ^ Should appear RED/YELLOW (priority 100 beats priority 10)\n\n")

  (let ((base (point)))
    (insert "Reversed creation order: high-priority created first.\n")
    ;; High priority created first: magenta
    (overlay-test--make-ov base (point)
                           'face '(:background "dark magenta" :foreground "white")
                           'priority 200)
    ;; Low priority created second: cyan
    (overlay-test--make-ov base (point)
                           'face '(:background "dark cyan" :foreground "white")
                           'priority 5))
  (insert "  ^ Should appear MAGENTA (priority 200 beats priority 5)\n\n")

  (insert "Three overlays, ascending priority:\n")
  (let ((base (point)))
    (insert "The highest priority face should dominate here.\n")
    (overlay-test--make-ov base (point)
                           'face '(:background "dark green")
                           'priority 1)
    (overlay-test--make-ov base (point)
                           'face '(:background "dark blue")
                           'priority 50)
    (overlay-test--make-ov base (point)
                           'face '(:background "dark red" :foreground "white" :weight bold)
                           'priority 999))
  (insert "  ^ Should appear DARK RED, bold, white text (priority 999)\n")

  (goto-char (point-min))
  (message "Test 7: Overlay priority — higher priority wins"))

;; ============================================================================
;; Test 8: Propertized before-string and after-string
;; ============================================================================

(defun overlay-test--propertized-strings ()
  "Test before-string and after-string with their own face properties."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 8: Propertized before/after-string")

  (let ((start (point)))
    (insert "Target text for propertized strings.\n")
    (overlay-test--make-ov start (point)
                           'before-string (propertize "[PRE] "
                                                      'face '(:background "dark magenta"
                                                              :foreground "white"
                                                              :weight bold))
                           'after-string (propertize " [POST]"
                                                     'face '(:background "dark cyan"
                                                             :foreground "white"
                                                             :slant italic))))

  (insert "\n")
  (let ((start (point)))
    (insert "Another line with fancy decorations.\n")
    (overlay-test--make-ov start (point)
                           'before-string (propertize ">> "
                                                      'face '(:foreground "gold"
                                                              :height 1.3
                                                              :weight bold))
                           'after-string (propertize " <<"
                                                     'face '(:foreground "gold"
                                                             :height 1.3
                                                             :weight bold))))

  (insert "\nMulti-face before-string:\n")
  (let ((start (point)))
    (insert "Line with a composite before-string.\n")
    (let ((bs (concat (propertize "ERR" 'face '(:background "red" :foreground "white" :weight bold))
                      (propertize ":" 'face '(:foreground "gray"))
                      (propertize " " 'face 'default))))
      (overlay-test--make-ov start (point)
                             'before-string bs)))

  (goto-char (point-min))
  (message "Test 8: Propertized before/after-string — faces on strings themselves"))

;; ============================================================================
;; Test 9: Overlay spanning multiple lines
;; ============================================================================

(defun overlay-test--multiline ()
  "Test overlay that spans across multiple lines."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 9: Multi-line Overlay")

  (insert "Text before the multi-line overlay.\n")
  (let ((start (point)))
    (insert "First line of multi-line overlay region.\n")
    (insert "Second line of multi-line overlay region.\n")
    (insert "Third line of multi-line overlay region.\n")
    (insert "Fourth line of multi-line overlay region.\n")
    (insert "Fifth line — end of overlay region.\n")
    (overlay-test--make-ov start (point)
                           'face '(:background "#2a2a4a" :foreground "light sky blue")))

  (insert "Text after the multi-line overlay.\n\n")

  (insert "Multi-line overlay with before/after-string:\n")
  (let ((start (point)))
    (insert "Line 1 inside block.\n")
    (insert "Line 2 inside block.\n")
    (insert "Line 3 inside block.\n")
    (overlay-test--make-ov start (point)
                           'face '(:background "#3a2a2a")
                           'before-string (propertize "--- BLOCK START ---\n"
                                                      'face '(:foreground "salmon" :weight bold))
                           'after-string (propertize "--- BLOCK END ---\n"
                                                     'face '(:foreground "salmon" :weight bold))))

  (goto-char (point-min))
  (message "Test 9: Multi-line overlay — spanning five lines with background"))

;; ============================================================================
;; Test 10: Overlay with `evaporate' property
;; ============================================================================

(defun overlay-test--evaporate ()
  "Test overlay evaporate property — overlay deleted when region becomes empty."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 10: Overlay evaporate Property")

  (insert "The overlay below has `evaporate' set to t.\n")
  (insert "Delete the highlighted text and the overlay should vanish automatically.\n\n")

  (let ((start (point)))
    (insert "DELETE THIS TEXT")
    (let ((end (point)))
      (insert "\n")
      (let ((ov (overlay-test--make-ov start end
                                       'face '(:background "dark red" :foreground "yellow" :weight bold)
                                       'evaporate t)))
        (insert (format "\nOverlay exists: %s\n" (if (overlay-buffer ov) "YES" "NO (evaporated)")))
        (insert "Number of overlays in buffer: "
                (number-to-string (length (overlays-in (point-min) (point-max))))
                "\n")
        (insert "\nInstructions:\n")
        (insert "  1. Select the red highlighted text above ('DELETE THIS TEXT')\n")
        (insert "  2. Delete it (C-w or Backspace)\n")
        (insert "  3. The overlay should automatically evaporate\n")
        (insert "  4. Check: (length (overlays-in (point-min) (point-max))) in *scratch*\n"))))

  (goto-char (point-min))
  (setq buffer-read-only nil)
  (message "Test 10: Overlay evaporate — delete highlighted text to see overlay disappear"))

;; ============================================================================
;; Test 11: hl-line-mode simulation
;; ============================================================================

(defun overlay-test--hl-line ()
  "Simulate hl-line-mode: highlight the current line with an overlay."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 11: hl-line-mode Simulation")

  (dotimes (i 20)
    (insert (format "Line %02d: The quick brown fox jumps over the lazy dog.\n" (1+ i))))

  ;; Create a line-highlight overlay on line 5
  (goto-char (point-min))
  (forward-line 5)
  (let ((bol (line-beginning-position))
        (eol (1+ (line-end-position))))
    (overlay-test--make-ov bol eol
                           'face '(:background "#333355")
                           'priority -50))

  ;; Also enable real hl-line-mode for comparison
  (hl-line-mode 1)

  (goto-char (point-min))
  (forward-line 10)
  (message "Test 11: hl-line — line 6 has manual overlay, real hl-line-mode also active"))

;; ============================================================================
;; Test 12: Simulated search highlight
;; ============================================================================

(defun overlay-test--search-highlight ()
  "Simulate isearch highlights: multiple overlays scattered through text."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 12: Search Highlight Simulation")

  ;; Insert sample text
  (let ((text "The fox jumped over the lazy dog. The fox ran through the forest.\n"))
    (dotimes (_i 15)
      (insert text)))

  ;; Highlight all occurrences of "fox" with lazy-highlight face
  (goto-char (point-min))
  (let ((count 0))
    (while (search-forward "fox" nil t)
      (let ((ov (overlay-test--make-ov (match-beginning 0) (match-end 0)
                                       'face (if (= count 0)
                                                  '(:background "yellow" :foreground "black" :weight bold)
                                                '(:background "dark goldenrod" :foreground "black"))
                                       'priority 1000)))
        ;; First match gets the "current match" highlight
        (when (= count 0)
          (overlay-put ov 'priority 1001)))
      (setq count (1+ count)))
    (insert (format "\nHighlighted %d occurrences of 'fox'.\n" count))
    (insert "First occurrence: bright yellow (current match).\n")
    (insert "Others: dark goldenrod (lazy highlight).\n"))

  (goto-char (point-min))
  (message "Test 12: Search highlight — 'fox' highlighted throughout text"))

;; ============================================================================
;; Test 13: Overlay with modification-hooks
;; ============================================================================

(defun overlay-test--modification-hooks ()
  "Test overlay modification-hooks — informational, shows hook registration."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 13: Overlay modification-hooks")

  (defvar overlay-test--hook-log nil
    "Log of modification hook invocations.")

  (setq overlay-test--hook-log nil)

  (defun overlay-test--mod-hook (ov after-p beg end &optional length)
    "Log when overlay OV region is modified."
    (push (format "%s: ov=%S region=%d-%d%s"
                  (if after-p "AFTER" "BEFORE")
                  (overlay-start ov)
                  beg end
                  (if after-p (format " len=%d" length) ""))
          overlay-test--hook-log)
    (message "Modification hook fired: %s" (car overlay-test--hook-log)))

  (insert "The text below has a modification-hook overlay.\n")
  (insert "Edit inside the highlighted region to trigger the hook.\n\n")

  (let ((start (point)))
    (insert "EDIT THIS TEXT TO TRIGGER HOOKS")
    (overlay-test--make-ov start (point)
                           'face '(:background "dark green" :foreground "white")
                           'modification-hooks '(overlay-test--mod-hook)
                           'insert-in-front-hooks '(overlay-test--mod-hook)
                           'insert-behind-hooks '(overlay-test--mod-hook)))

  (insert "\n\n")
  (insert "Modification hook properties set:\n")
  (insert "  - modification-hooks\n")
  (insert "  - insert-in-front-hooks\n")
  (insert "  - insert-behind-hooks\n")
  (insert "\nCheck *Messages* buffer for hook invocations after editing.\n")

  (goto-char (point-min))
  (setq buffer-read-only nil)
  (message "Test 13: modification-hooks — edit green region, check *Messages*"))

;; ============================================================================
;; Test 14: Overlay with line-prefix and wrap-prefix
;; ============================================================================

(defun overlay-test--prefix ()
  "Test overlay line-prefix and wrap-prefix for indentation."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Test 14: Overlay line-prefix and wrap-prefix")

  (insert "Lines below have overlay line-prefix and wrap-prefix set.\n")
  (insert "They should appear indented without actual spaces in the buffer.\n\n")

  ;; Line prefix: adds visual prefix before each line
  (let ((start (point)))
    (insert "First line with line-prefix.\n")
    (insert "Second line with line-prefix.\n")
    (insert "Third line with line-prefix.\n")
    (overlay-test--make-ov start (point)
                           'line-prefix (propertize "  | "
                                                    'face '(:foreground "gray50"))))

  (insert "\n")

  ;; Wrap prefix: indentation for wrapped long lines
  (let ((start (point)))
    (insert "This is a very long line that should wrap around the window edge. ")
    (insert "When it wraps, the continuation should be indented by the wrap-prefix. ")
    (insert "This helps visually distinguish wrapped lines from new lines. ")
    (insert "Keep reading to see the wrap-prefix in action on this long paragraph.\n")
    (overlay-test--make-ov start (point)
                           'wrap-prefix "        "
                           'face '(:background "#1a1a2a")))

  (insert "\n")

  ;; Both line-prefix and wrap-prefix together
  (let ((start (point)))
    (insert "Combined: line-prefix adds a bullet, wrap-prefix indents continuations. ")
    (insert "This long line should wrap and show the wrap-prefix indentation clearly. ")
    (insert "The bullet only appears at the start of each actual line.\n")
    (insert "Another line with the same combined prefix treatment.\n")
    (overlay-test--make-ov start (point)
                           'line-prefix (propertize "  * " 'face '(:foreground "gold"))
                           'wrap-prefix "      "))

  (goto-char (point-min))
  (message "Test 14: line-prefix/wrap-prefix — visual indentation via overlays"))

;; ============================================================================
;; Combined: All overlays at once
;; ============================================================================

(defun overlay-test--all-at-once ()
  "Show all overlay types in a single buffer for a comprehensive visual check."
  (overlay-test--setup-buffer)

  (let ((start (point)))
    (insert "OVERLAY RENDERING COMPREHENSIVE TEST\n")
    (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
  (insert (make-string 60 ?-) "\n\n")

  ;; Section 1: Face overlays
  (let ((s (point)))
    (insert "--- Face Overlays ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (let ((s (point)))
    (insert "Red background region.\n")
    (overlay-test--make-ov s (point)
                           'face '(:background "dark red" :foreground "white")))
  (let ((s (point)))
    (insert "Bold green text.\n")
    (overlay-test--make-ov s (point)
                           'face '(:foreground "green" :weight bold)))
  (let ((s (point)))
    (insert "Italic blue underlined.\n")
    (overlay-test--make-ov s (point)
                           'face '(:foreground "dodger blue" :slant italic :underline t)))
  (insert "\n")

  ;; Section 2: Before/after strings
  (let ((s (point)))
    (insert "--- Before/After Strings ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (let ((s (point)))
    (insert "Text with before and after strings.\n")
    (overlay-test--make-ov s (point)
                           'before-string (propertize "[B] " 'face '(:foreground "cyan" :weight bold))
                           'after-string (propertize " [A]" 'face '(:foreground "magenta" :weight bold))))
  (insert "\n")

  ;; Section 3: Display replacement
  (let ((s (point)))
    (insert "--- Display Replacement ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (let ((s (point)))
    (insert "ORIGINAL TEXT REPLACED")
    (overlay-test--make-ov s (point)
                           'display (propertize "[replaced via display]"
                                                'face '(:foreground "orchid" :weight bold))))
  (insert "\n\n")

  ;; Section 4: Invisible
  (let ((s (point)))
    (insert "--- Invisible Overlay ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (add-to-invisibility-spec '(overlay-test-all . t))
  (insert "before ")
  (let ((s (point)))
    (insert "HIDDEN")
    (overlay-test--make-ov s (point)
                           'invisible 'overlay-test-all))
  (insert " after (text between 'before' and 'after' is invisible with ellipsis)\n\n")

  ;; Section 5: Priority
  (let ((s (point)))
    (insert "--- Priority Test ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (let ((s (point)))
    (insert "High priority red beats low priority green.\n")
    (overlay-test--make-ov s (point)
                           'face '(:background "dark green")
                           'priority 1)
    (overlay-test--make-ov s (point)
                           'face '(:background "dark red" :foreground "white")
                           'priority 100))
  (insert "\n")

  ;; Section 6: Multi-line
  (let ((s (point)))
    (insert "--- Multi-line Overlay ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (let ((s (point)))
    (insert "Multi-line 1\n")
    (insert "Multi-line 2\n")
    (insert "Multi-line 3\n")
    (overlay-test--make-ov s (point)
                           'face '(:background "#2a2a3a")))

  ;; Section 7: Search highlights
  (let ((s (point)))
    (insert "--- Search Highlights ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (insert "The fox found a fox friend near the fox den.\n")
  (save-excursion
    (goto-char (point-min))
    (let ((first t))
      (while (search-forward "fox" nil t)
        (overlay-test--make-ov (match-beginning 0) (match-end 0)
                               'face (if first
                                         '(:background "yellow" :foreground "black" :weight bold)
                                       '(:background "dark goldenrod" :foreground "black"))
                               'priority 1000)
        (setq first nil))))
  (insert "\n")

  ;; Section 8: Line prefix
  (let ((s (point)))
    (insert "--- Line Prefix ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (let ((s (point)))
    (insert "Prefixed line one.\n")
    (insert "Prefixed line two.\n")
    (overlay-test--make-ov s (point)
                           'line-prefix (propertize "  > " 'face '(:foreground "gray50"))))

  ;; Section 9: hl-line simulation
  (let ((s (point)))
    (insert "--- hl-line Simulation ---\n")
    (put-text-property s (point) 'face '(:weight bold :foreground "gold")))
  (insert "Normal line.\n")
  (let ((s (point)))
    (insert "This line is highlighted like hl-line-mode.\n")
    (overlay-test--make-ov s (point)
                           'face '(:background "#333355")
                           'priority -50))
  (insert "Normal line.\n")

  (insert "\n" (make-string 60 ?-) "\n")
  (insert "All overlay types shown above. Scroll to review.\n")

  (goto-char (point-min))
  (message "All overlays shown — comprehensive visual test"))

;; ============================================================================
;; Interactive toggling
;; ============================================================================

(defvar overlay-test--toggle-overlays nil
  "Overlays used for the toggle test.")

(defvar overlay-test--toggle-state t
  "Current visibility state for toggle test overlays.")

(defun overlay-test--toggle-setup ()
  "Set up the toggle test buffer."
  (overlay-test--setup-buffer)
  (overlay-test--insert-heading "Interactive Overlay Toggle")

  (insert "Press 't' to toggle overlays on/off.\n")
  (insert "Press 'f' to cycle face styles.\n\n")

  (dotimes (i 10)
    (insert (format "Line %02d: Sample text for toggling overlays on and off.\n" (1+ i))))

  ;; Create overlays on even-numbered lines
  (setq overlay-test--toggle-overlays nil)
  (setq overlay-test--toggle-state t)
  (goto-char (point-min))
  (forward-line 4) ;; skip header
  (dotimes (i 10)
    (when (= (mod i 2) 0)
      (let ((bol (line-beginning-position))
            (eol (1+ (line-end-position))))
        (push (overlay-test--make-ov bol eol
                                     'face '(:background "#2a2a4a")
                                     'overlay-test-toggle t)
              overlay-test--toggle-overlays)))
    (forward-line 1))

  (goto-char (point-min))
  (message "Toggle test ready — press 't' to toggle, 'f' to cycle faces"))

(defvar overlay-test--face-cycle 0
  "Current face index for the cycle test.")

(defvar overlay-test--face-list
  '((:background "#2a2a4a")
    (:background "dark red" :foreground "white")
    (:background "dark green" :foreground "white")
    (:background "dark blue" :foreground "yellow")
    (:background "dark magenta" :foreground "white" :weight bold)
    (:foreground "orange" :underline t :slant italic))
  "List of faces to cycle through.")

(defun overlay-test--toggle ()
  "Toggle overlay visibility."
  (interactive)
  (setq overlay-test--toggle-state (not overlay-test--toggle-state))
  (dolist (ov overlay-test--toggle-overlays)
    (when (overlay-buffer ov)
      (if overlay-test--toggle-state
          (overlay-put ov 'face (nth overlay-test--face-cycle overlay-test--face-list))
        (overlay-put ov 'face nil))))
  (message "Overlays %s" (if overlay-test--toggle-state "ON" "OFF")))

(defun overlay-test--cycle-face ()
  "Cycle through different face styles for toggle overlays."
  (interactive)
  (setq overlay-test--face-cycle
        (mod (1+ overlay-test--face-cycle) (length overlay-test--face-list)))
  (let ((face (nth overlay-test--face-cycle overlay-test--face-list)))
    (dolist (ov overlay-test--toggle-overlays)
      (when (overlay-buffer ov)
        (overlay-put ov 'face face)))
    (setq overlay-test--toggle-state t)
    (message "Face style %d/%d: %S"
             (1+ overlay-test--face-cycle)
             (length overlay-test--face-list)
             face)))

;; ============================================================================
;; Test runner
;; ============================================================================

(defvar overlay-test--tests
  '(("Face property"              . overlay-test--face)
    ("Before-string"              . overlay-test--before-string)
    ("After-string"               . overlay-test--after-string)
    ("Display property"           . overlay-test--display)
    ("Invisible property"         . overlay-test--invisible)
    ("Overlapping overlays"       . overlay-test--overlapping)
    ("Priority"                   . overlay-test--priority)
    ("Propertized strings"        . overlay-test--propertized-strings)
    ("Multi-line overlay"         . overlay-test--multiline)
    ("Evaporate"                  . overlay-test--evaporate)
    ("hl-line simulation"         . overlay-test--hl-line)
    ("Search highlight"           . overlay-test--search-highlight)
    ("Modification hooks"         . overlay-test--modification-hooks)
    ("Line/wrap prefix"           . overlay-test--prefix))
  "Alist of (name . function) for overlay tests.")

(defun overlay-test-next ()
  "Run the next test in the sequence."
  (interactive)
  (when (>= overlay-test--step (length overlay-test--tests))
    (setq overlay-test--step 0))
  (let ((test (nth overlay-test--step overlay-test--tests)))
    (message "--- Running test %d/%d: %s ---"
             (1+ overlay-test--step)
             (length overlay-test--tests)
             (car test))
    (funcall (cdr test))
    (setq overlay-test--step (1+ overlay-test--step))))

(defun overlay-test-run-all ()
  "Run all tests sequentially with pauses."
  (interactive)
  (setq overlay-test--step 0)
  (let ((i 0))
    (dolist (test overlay-test--tests)
      (run-at-time (* i 3) nil
                   (lambda (test-pair idx)
                     (message "\n=== Test %d/%d: %s ==="
                              (1+ idx)
                              (length overlay-test--tests)
                              (car test-pair))
                     (funcall (cdr test-pair)))
                   test i)
      (setq i (1+ i)))
    (run-at-time (* i 3) nil
                 (lambda ()
                   (message "\n=== All %d overlay tests complete ==="
                            (length overlay-test--tests))
                   (message "Press 'c' for combined view, 'i' for interactive toggle"))))
  (message "Running all %d tests (3 second intervals)..." (length overlay-test--tests)))

;; Keybindings for interactive use
(defvar overlay-test-map (make-sparse-keymap)
  "Keymap for overlay test.")

(define-key overlay-test-map (kbd "n") #'overlay-test-next)
(define-key overlay-test-map (kbd "a") #'overlay-test-run-all)
(define-key overlay-test-map (kbd "c") (lambda () (interactive) (overlay-test--all-at-once)))
(define-key overlay-test-map (kbd "i") (lambda () (interactive) (overlay-test--toggle-setup)))
(define-key overlay-test-map (kbd "t") #'overlay-test--toggle)
(define-key overlay-test-map (kbd "f") #'overlay-test--cycle-face)
(define-key overlay-test-map (kbd "q") (lambda () (interactive)
                                         (overlay-test--cleanup)
                                         (message "All test overlays cleaned up.")))
(define-key overlay-test-map (kbd "1")  (lambda () (interactive) (overlay-test--face)))
(define-key overlay-test-map (kbd "2")  (lambda () (interactive) (overlay-test--before-string)))
(define-key overlay-test-map (kbd "3")  (lambda () (interactive) (overlay-test--after-string)))
(define-key overlay-test-map (kbd "4")  (lambda () (interactive) (overlay-test--display)))
(define-key overlay-test-map (kbd "5")  (lambda () (interactive) (overlay-test--invisible)))
(define-key overlay-test-map (kbd "6")  (lambda () (interactive) (overlay-test--overlapping)))
(define-key overlay-test-map (kbd "7")  (lambda () (interactive) (overlay-test--priority)))
(define-key overlay-test-map (kbd "8")  (lambda () (interactive) (overlay-test--propertized-strings)))
(define-key overlay-test-map (kbd "9")  (lambda () (interactive) (overlay-test--multiline)))
(define-key overlay-test-map (kbd "0")  (lambda () (interactive) (overlay-test--evaporate)))

(set-transient-map overlay-test-map t)

;; Setup on load
(message "=== Overlay Rendering Test Suite ===")
(message "Keys: n=next  a=run-all  c=combined  i=interactive-toggle  1-0=specific  q=cleanup")
(overlay-test--face)

;;; overlay-test.el ends here
