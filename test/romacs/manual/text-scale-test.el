;;; text-scale-test.el --- Test text scaling and variable-height faces -*- lexical-binding: t -*-

;; Test text-scale-adjust, variable-height face rendering, cursor sizing,
;; line-number scaling, word-wrap with mixed heights, face inheritance,
;; and mode-line custom face heights in the romacs GPU renderer.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/text-scale-test.el

;;; Code:

(defvar text-scale-test--step 0
  "Current test step for the automated sequence.")

(defvar text-scale-test--timer nil
  "Timer for cycling text-scale levels.")

(defvar text-scale-test--cycle-step 0
  "Current step in the text-scale cycling animation.")

;; ============================================================================
;; Helpers
;; ============================================================================

(defun text-scale-test--insert-heading (title)
  "Insert section TITLE in bold gold."
  (let ((start (point)))
    (insert (format "\n=== %s ===\n\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold" :height 1.2))))

(defun text-scale-test--fill-buffer (buf text)
  "In buffer BUF, erase and insert TEXT."
  (with-current-buffer buf
    (erase-buffer)
    (insert text)
    (goto-char (point-min))))

;; ============================================================================
;; Test 1: text-scale-adjust at various levels (-3 to +3) in side-by-side windows
;; ============================================================================

(defun text-scale-test--scale-levels ()
  "Show text-scale-adjust levels -3 through +3 in separate windows."
  (delete-other-windows)
  (let ((levels '(-3 -2 -1 0 1 2 3))
        (sample-text "The quick brown fox jumps over the lazy dog.\nPack my box with five dozen liquor jugs.\nHow vexingly quick daft zebras jump!\n")
        windows)
    ;; Create buffers and fill them
    (dolist (level levels)
      (let ((buf (get-buffer-create (format "*scale:%+d*" level))))
        (with-current-buffer buf
          (erase-buffer)
          (let ((start (point)))
            (insert (format "text-scale-adjust: %+d\n" level))
            (put-text-property start (point) 'face '(:weight bold :foreground "cyan")))
          (insert (make-string 40 ?-) "\n")
          (dotimes (i 8)
            (insert (format "Line %d: %s" (1+ i) sample-text)))
          (goto-char (point-min))
          ;; Apply text-scale
          (text-scale-set level))))
    ;; Arrange windows: split into 7 vertical columns
    (switch-to-buffer "*scale: 0*")
    ;; Show 4 levels side by side (more than 7 is too narrow)
    (let ((selected-levels '(-3 -1 0 +1 +3)))
      (switch-to-buffer (get-buffer-create (format "*scale:%+d*" (car selected-levels))))
      (dolist (level (cdr selected-levels))
        (split-window-right)
        (other-window 1)
        (switch-to-buffer (get-buffer-create (format "*scale:%+d*" level))))
      (balance-windows)
      (other-window 1)))
  (message "Test 1: text-scale-adjust levels -3, -1, 0, +1, +3 side by side"))

;; ============================================================================
;; Test 2: Face :height with absolute pixel sizes
;; ============================================================================

(defun text-scale-test--absolute-heights ()
  "Test face :height with absolute pixel sizes."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*absolute-heights*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Face :height — Absolute Pixel Sizes")
    (let ((sizes '(100 140 200 280)))
      (dolist (size sizes)
        (let ((start (point)))
          (insert (format "Height %d: " size))
          (put-text-property start (point) 'face `(:foreground "white" :height ,size)))
        (let ((start (point)))
          (insert "The quick brown fox jumps.\n")
          (put-text-property start (point) 'face `(:foreground "light green" :height ,size)))))
    (insert "\n")
    (insert "Each line above should be progressively taller.\n")
    (insert "100 = small, 140 = normal, 200 = large, 280 = very large.\n")
    (goto-char (point-min)))
  (message "Test 2: Absolute pixel heights — 100, 140, 200, 280"))

;; ============================================================================
;; Test 3: Face :height with relative float values
;; ============================================================================

(defun text-scale-test--relative-heights ()
  "Test face :height with relative float values."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*relative-heights*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Face :height — Relative Float Values")
    (let ((scales '(0.5 0.8 1.0 1.2 1.5 2.0 3.0)))
      (dolist (scale scales)
        (let ((start (point)))
          (insert (format "%.1fx: " scale))
          (put-text-property start (point) 'face `(:foreground "white" :height ,scale)))
        (let ((start (point)))
          (insert "ABCDEFghijklm 0123456789\n")
          (put-text-property start (point) 'face `(:foreground "sky blue" :height ,scale)))))
    (insert "\n")
    (insert "0.5x = half size, 1.0x = normal, 2.0x = double, 3.0x = triple.\n")
    (insert "Row heights should grow to accommodate the tallest glyph.\n")
    (goto-char (point-min)))
  (message "Test 3: Relative float heights — 0.5, 0.8, 1.0, 1.2, 1.5, 2.0, 3.0"))

;; ============================================================================
;; Test 4: Mixed heights on the same line
;; ============================================================================

(defun text-scale-test--mixed-heights ()
  "Test mixed face heights on the same line."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*mixed-heights*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Mixed Heights on Same Line")

    ;; Line 1: small next to large
    (let ((s (point)))
      (insert "tiny ")
      (put-text-property s (point) 'face '(:height 0.5 :foreground "salmon")))
    (let ((s (point)))
      (insert "HUGE ")
      (put-text-property s (point) 'face '(:height 3.0 :foreground "cyan" :weight bold)))
    (let ((s (point)))
      (insert "normal ")
      (put-text-property s (point) 'face '(:foreground "white")))
    (let ((s (point)))
      (insert "medium")
      (put-text-property s (point) 'face '(:height 1.5 :foreground "light green")))
    (insert "\n")

    ;; Line 2: gradient of sizes
    (dolist (h '(0.5 0.7 0.9 1.1 1.3 1.5 1.8 2.0 2.5 3.0))
      (let ((s (point)))
        (insert (format "%.1f " h))
        (put-text-property s (point) 'face `(:height ,h :foreground "orange"))))
    (insert "\n")

    ;; Line 3: alternating small and large
    (dotimes (i 10)
      (let ((s (point))
            (big (= (mod i 2) 0)))
        (insert (if big "BIG " "sm "))
        (put-text-property s (point) 'face
                           `(:height ,(if big 2.0 0.7)
                             :foreground ,(if big "yellow" "gray")))))
    (insert "\n")

    ;; Line 4: one huge character among normal text
    (insert "Normal text with a ")
    (let ((s (point)))
      (insert "G")
      (put-text-property s (point) 'face '(:height 3.0 :foreground "red" :weight bold)))
    (insert "iant letter in the middle.\n")

    (insert "\nRow heights should accommodate the tallest glyph on each line.\n")
    (insert "Baselines should align correctly for different sized text.\n")
    (goto-char (point-min)))
  (message "Test 4: Mixed heights on same line — small next to large"))

;; ============================================================================
;; Test 5: Variable-height text with line numbers
;; ============================================================================

(defun text-scale-test--line-numbers ()
  "Test line numbers with variable-height text."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*line-numbers-height*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Variable-Height Text + Line Numbers")

    (dotimes (i 20)
      (let ((h (nth (mod i 5) '(0.8 1.0 1.3 1.8 2.5)))
            (s (point)))
        (insert (format "Line at height %.1f: The quick brown fox jumps over the lazy dog." h))
        (put-text-property s (point) 'face `(:height ,h :foreground "light steel blue"))
        (insert "\n")))

    (insert "\n")
    (let ((s (point)))
      (insert "Line numbers should scale to match each row's height.\n")
      (insert "No overlapping between line-number column and text.\n")
      (put-text-property s (point) 'face '(:foreground "gray")))

    (goto-char (point-min))
    (display-line-numbers-mode 1))
  (message "Test 5: Variable-height text with display-line-numbers-mode"))

;; ============================================================================
;; Test 6: Variable-height text with word-wrap
;; ============================================================================

(defun text-scale-test--word-wrap ()
  "Test word-wrap with variable-height faces."
  (delete-other-windows)
  ;; Use a narrower window to force wrapping
  (split-window-right)
  (let ((buf (get-buffer-create "*word-wrap-height*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Word-Wrap + Variable Heights")

    ;; Long line with mixed heights that should wrap
    (let ((s (point)))
      (insert "This is normal text that flows along. ")
      (put-text-property s (point) 'face '(:foreground "white")))
    (let ((s (point)))
      (insert "THIS PART IS TALL (2x) AND SHOULD CAUSE THE WRAPPED ROW TO BE TALLER. ")
      (put-text-property s (point) 'face '(:height 2.0 :foreground "orange" :weight bold)))
    (let ((s (point)))
      (insert "Back to normal size text continuing on, which should wrap at the window edge. ")
      (put-text-property s (point) 'face '(:foreground "white")))
    (let ((s (point)))
      (insert "HUGE TEXT (3x) wrapping here too! ")
      (put-text-property s (point) 'face '(:height 3.0 :foreground "red")))
    (let ((s (point)))
      (insert "And finally small text at 0.7x to finish this paragraph.")
      (put-text-property s (point) 'face '(:height 0.7 :foreground "gray")))
    (insert "\n\n")

    ;; Another paragraph
    (let ((s (point)))
      (insert "Second paragraph: Each word has a different height for stress testing. ")
      (put-text-property s (point) 'face '(:foreground "cyan")))
    (dolist (pair '(("alpha " . 0.6) ("BETA " . 1.8) ("gamma " . 0.9)
                    ("DELTA " . 2.5) ("epsilon " . 1.0) ("ZETA " . 1.5)
                    ("eta " . 0.7) ("THETA " . 2.0) ("iota " . 1.1)
                    ("KAPPA " . 3.0)))
      (let ((s (point)))
        (insert (car pair))
        (put-text-property s (point) 'face `(:height ,(cdr pair) :foreground "light green"))))
    (insert "\n")

    (insert "\nWrapped rows should have height matching their tallest glyph.\n")
    (setq truncate-lines nil)
    (setq word-wrap t)
    (goto-char (point-min)))

  ;; Show a normal buffer in the other window for comparison
  (other-window 1)
  (switch-to-buffer (get-buffer-create "*wrap-compare*"))
  (erase-buffer)
  (insert "This is a normal-height comparison window.\n")
  (insert "Text here is at default size for reference.\n")
  (dotimes (i 20)
    (insert (format "Comparison line %d\n" (1+ i))))
  (goto-char (point-min))
  (other-window 1)
  (message "Test 6: Word-wrap with variable-height faces (narrow window)"))

;; ============================================================================
;; Test 7: Cursor positioning with variable-height faces
;; ============================================================================

(defun text-scale-test--cursor ()
  "Test cursor sizing with variable-height faces."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*cursor-height*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Cursor Positioning with Variable-Height Faces")

    (insert "Move the cursor across these lines. The cursor height should\n")
    (insert "match the face height at each position.\n\n")

    ;; Line with segments of different heights
    (let ((segments '(("Normal" nil "white")
                      (" Tiny" 0.5 "salmon")
                      (" Small" 0.8 "khaki")
                      (" Medium" 1.2 "light green")
                      (" Large" 1.5 "sky blue")
                      (" Huge" 2.0 "orange")
                      (" Giant" 3.0 "red"))))
      (dolist (seg segments)
        (let ((s (point))
              (text (nth 0 seg))
              (height (nth 1 seg))
              (color (nth 2 seg)))
          (insert text)
          (if height
              (put-text-property s (point) 'face `(:height ,height :foreground ,color))
            (put-text-property s (point) 'face `(:foreground ,color))))))
    (insert "\n\n")

    ;; Rows of uniform height for easy cursor-height checking
    (dolist (h '(0.5 1.0 1.5 2.0 3.0))
      (let ((s (point)))
        (insert (format "Cursor here at height %.1f: >>>         <<<" h))
        (put-text-property s (point) 'face `(:height ,h :foreground "light steel blue")))
      (insert "\n"))

    (insert "\nUse arrow keys to move cursor. Cursor height should change\n")
    (insert "to match the face at each character position.\n")
    (insert "Filled box cursor = face height. Bar cursor = face height.\n")
    (goto-char (point-min)))
  (message "Test 7: Cursor positioning — move cursor across variable-height text"))

;; ============================================================================
;; Test 8: Face inheritance with height
;; ============================================================================

(defun text-scale-test--inheritance ()
  "Test face inheritance: parent with large height, child with relative."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*face-inherit*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Face Inheritance with :height")

    ;; Define test faces
    (defface text-scale-test-parent
      '((t :height 2.0 :foreground "gold" :weight bold))
      "Parent face with large height for testing.")

    (defface text-scale-test-child-relative
      '((t :inherit text-scale-test-parent :height 0.5 :foreground "sky blue"))
      "Child face: inherits parent 2.0x, own 0.5x = effective 1.0x.")

    (defface text-scale-test-child-absolute
      '((t :inherit text-scale-test-parent :height 100 :foreground "salmon"))
      "Child face: inherits parent but overrides with absolute 100.")

    (defface text-scale-test-grandchild
      '((t :inherit text-scale-test-child-relative :height 1.5 :foreground "light green"))
      "Grandchild: inherits child (effective 1.0x), own 1.5x = effective 1.5x.")

    ;; Show parent face
    (insert "Parent face (2.0x): ")
    (let ((s (point)))
      (insert "PARENT HEIGHT TEXT\n")
      (put-text-property s (point) 'face 'text-scale-test-parent))

    ;; Show child with relative height
    (insert "Child relative (inherit 2.0x, own 0.5x => 1.0x): ")
    (let ((s (point)))
      (insert "CHILD RELATIVE TEXT\n")
      (put-text-property s (point) 'face 'text-scale-test-child-relative))

    ;; Show child with absolute height
    (insert "Child absolute (inherit 2.0x, own 100px override): ")
    (let ((s (point)))
      (insert "CHILD ABSOLUTE TEXT\n")
      (put-text-property s (point) 'face 'text-scale-test-child-absolute))

    ;; Show grandchild
    (insert "Grandchild (chain: 2.0 * 0.5 * 1.5 => 1.5x): ")
    (let ((s (point)))
      (insert "GRANDCHILD TEXT\n")
      (put-text-property s (point) 'face 'text-scale-test-grandchild))

    ;; Normal reference
    (insert "\nNormal text for reference (1.0x default height).\n")

    (insert "\nFace inheritance chain resolves heights multiplicatively.\n")
    (insert "Absolute pixel size overrides the chain.\n")
    (goto-char (point-min)))
  (message "Test 8: Face inheritance — parent 2.0x, child relative 0.5x, grandchild 1.5x"))

;; ============================================================================
;; Test 9: text-scale-mode in multiple windows at different levels
;; ============================================================================

(defun text-scale-test--multi-window ()
  "Show text-scale-mode at different levels in multiple windows."
  (delete-other-windows)
  (let* ((levels '(-2 0 +2 +4))
         (sample (concat
                  "The quick brown fox jumps over the lazy dog.\n"
                  "Pack my box with five dozen liquor jugs.\n"
                  "How vexingly quick daft zebras jump!\n"
                  "\n"
                  "(defun fibonacci (n)\n"
                  "  (if (< n 2) n\n"
                  "    (+ (fibonacci (1- n))\n"
                  "       (fibonacci (- n 2)))))\n"
                  "\n")))
    ;; Create a 2x2 grid
    (split-window-below)
    (split-window-right)
    (other-window 2)
    (split-window-right)
    (other-window -2)
    ;; Fill each quadrant
    (let ((wins (list (selected-window))))
      (dotimes (_ 3)
        (other-window 1)
        (push (selected-window) wins))
      (setq wins (nreverse wins))
      (dotimes (i 4)
        (let ((win (nth i wins))
              (level (nth i levels))
              (buf (get-buffer-create (format "*multi-scale:%+d*" (nth i levels)))))
          (set-window-buffer win buf)
          (with-current-buffer buf
            (erase-buffer)
            (let ((s (point)))
              (insert (format "text-scale: %+d\n" level))
              (put-text-property s (point) 'face '(:weight bold :foreground "cyan")))
            (insert (make-string 36 ?-) "\n")
            (dotimes (_ 5)
              (insert sample))
            (goto-char (point-min))
            (text-scale-set level))))))
  (message "Test 9: text-scale-mode in 2x2 grid at levels -2, 0, +2, +4"))

;; ============================================================================
;; Test 10: Mode-line with custom face heights
;; ============================================================================

(defun text-scale-test--mode-line ()
  "Test mode-line rendering with custom face heights."
  (delete-other-windows)

  ;; Define a custom mode-line face with larger height
  (defface text-scale-test-mode-line-big
    '((t :inherit mode-line :height 1.3 :weight bold))
    "Taller mode-line face for testing.")

  (defface text-scale-test-mode-line-small
    '((t :inherit mode-line :height 0.8))
    "Smaller mode-line face for testing.")

  (split-window-below)

  ;; Top window: big mode-line face
  (let ((buf (get-buffer-create "*modeline-big*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Mode-Line with :height 1.3")
    (insert "This window's mode-line uses a taller face.\n")
    (insert "The mode-line text should be visibly larger than default.\n")
    (dotimes (i 10)
      (insert (format "Content line %d\n" (1+ i))))
    (setq-local mode-line-format
                `((:propertize " %b " face text-scale-test-mode-line-big)
                  (:propertize " [Big Mode-Line] " face text-scale-test-mode-line-big)
                  (:propertize " L%l C%c " face text-scale-test-mode-line-big)))
    (goto-char (point-min))
    (force-mode-line-update))

  ;; Bottom window: small mode-line face
  (other-window 1)
  (let ((buf (get-buffer-create "*modeline-small*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (text-scale-test--insert-heading "Mode-Line with :height 0.8")
    (insert "This window's mode-line uses a smaller face.\n")
    (insert "The mode-line text should be visibly smaller than default.\n")
    (dotimes (i 10)
      (insert (format "Content line %d\n" (1+ i))))
    (setq-local mode-line-format
                `((:propertize " %b " face text-scale-test-mode-line-small)
                  (:propertize " [Small Mode-Line] " face text-scale-test-mode-line-small)
                  (:propertize " L%l C%c " face text-scale-test-mode-line-small)))
    (goto-char (point-min))
    (force-mode-line-update))
  (other-window 1)
  (message "Test 10: Mode-line faces — top=1.3x height, bottom=0.8x height"))

;; ============================================================================
;; Bonus: Cycling animation — watch text-scale transition smoothly
;; ============================================================================

(defun text-scale-test--start-cycle ()
  "Cycle through text-scale levels with `run-at-time' to observe transitions."
  (interactive)
  (delete-other-windows)
  (let ((buf (get-buffer-create "*scale-cycle*")))
    (switch-to-buffer buf)
    (erase-buffer)
    (let ((s (point)))
      (insert "TEXT SCALE CYCLING ANIMATION\n")
      (put-text-property s (point) 'face '(:weight bold :foreground "gold" :height 1.5)))
    (insert (make-string 50 ?-) "\n\n")
    (insert "Watch the text size change every 1.5 seconds.\n")
    (insert "Levels cycle: -3, -2, -1, 0, +1, +2, +3, +2, +1, 0, -1, -2\n\n")
    (dotimes (i 30)
      (insert (format "Line %02d: The quick brown fox jumps over the lazy dog.\n" (1+ i))))
    (goto-char (point-min)))
  ;; Cancel any existing timer
  (when text-scale-test--timer
    (cancel-timer text-scale-test--timer)
    (setq text-scale-test--timer nil))
  ;; Cycle sequence: ramp up and down
  (let ((levels '(-3 -2 -1 0 1 2 3 2 1 0 -1 -2)))
    (setq text-scale-test--cycle-step 0)
    (setq text-scale-test--timer
          (run-at-time 0 1.5
                       (lambda ()
                         (when (buffer-live-p (get-buffer "*scale-cycle*"))
                           (with-current-buffer "*scale-cycle*"
                             (let* ((levels '(-3 -2 -1 0 1 2 3 2 1 0 -1 -2))
                                    (level (nth (mod text-scale-test--cycle-step
                                                     (length levels))
                                                levels)))
                               (text-scale-set level)
                               (message "Scale cycle: %+d (step %d)"
                                        level text-scale-test--cycle-step)
                               (setq text-scale-test--cycle-step
                                     (1+ text-scale-test--cycle-step))))))))))

(defun text-scale-test--stop-cycle ()
  "Stop the cycling animation."
  (interactive)
  (when text-scale-test--timer
    (cancel-timer text-scale-test--timer)
    (setq text-scale-test--timer nil)
    (message "Scale cycling stopped.")))

;; ============================================================================
;; Test runner
;; ============================================================================

(defvar text-scale-test--tests
  '(("Scale levels side-by-side"       . text-scale-test--scale-levels)
    ("Absolute pixel heights"           . text-scale-test--absolute-heights)
    ("Relative float heights"           . text-scale-test--relative-heights)
    ("Mixed heights on same line"       . text-scale-test--mixed-heights)
    ("Line numbers + variable height"   . text-scale-test--line-numbers)
    ("Word-wrap + variable height"      . text-scale-test--word-wrap)
    ("Cursor positioning"               . text-scale-test--cursor)
    ("Face inheritance with :height"    . text-scale-test--inheritance)
    ("Multi-window text-scale-mode"     . text-scale-test--multi-window)
    ("Mode-line custom face heights"    . text-scale-test--mode-line))
  "Alist of (name . function) for text-scale tests.")

(defun text-scale-test-next ()
  "Run the next test in the sequence."
  (interactive)
  (text-scale-test--stop-cycle)
  (when (>= text-scale-test--step (length text-scale-test--tests))
    (setq text-scale-test--step 0))
  (let ((test (nth text-scale-test--step text-scale-test--tests)))
    (message "--- Running test %d/%d: %s ---"
             (1+ text-scale-test--step)
             (length text-scale-test--tests)
             (car test))
    (funcall (cdr test))
    (setq text-scale-test--step (1+ text-scale-test--step))))

(defun text-scale-test-run-all ()
  "Run all tests sequentially with pauses."
  (interactive)
  (text-scale-test--stop-cycle)
  (setq text-scale-test--step 0)
  (let ((i 0))
    (dolist (test text-scale-test--tests)
      (run-at-time (* i 4) nil
                   (lambda (test-pair idx)
                     (message "\n=== Test %d/%d: %s ==="
                              (1+ idx)
                              (length text-scale-test--tests)
                              (car test-pair))
                     (funcall (cdr test-pair)))
                   test i)
      (setq i (1+ i)))
    ;; Completion message
    (run-at-time (* i 4) nil
                 (lambda ()
                   (message "\n=== All %d text-scale tests complete ===" (length text-scale-test--tests))
                   (message "Press 'n' for next, 'a' to rerun all, 'c' to cycle, 's' to stop cycle, 'q' to quit"))))
  (message "Running all %d tests (4 second intervals)..." (length text-scale-test--tests)))

;; Keybindings for interactive use
(defvar text-scale-test-map (make-sparse-keymap)
  "Keymap for text-scale test.")

(define-key text-scale-test-map (kbd "n") #'text-scale-test-next)
(define-key text-scale-test-map (kbd "a") #'text-scale-test-run-all)
(define-key text-scale-test-map (kbd "c") #'text-scale-test--start-cycle)
(define-key text-scale-test-map (kbd "s") #'text-scale-test--stop-cycle)
(define-key text-scale-test-map (kbd "q") (lambda () (interactive)
                                            (text-scale-test--stop-cycle)
                                            (message "Text-scale tests finished.")))
(define-key text-scale-test-map (kbd "1") (lambda () (interactive) (text-scale-test--scale-levels)))
(define-key text-scale-test-map (kbd "2") (lambda () (interactive) (text-scale-test--absolute-heights)))
(define-key text-scale-test-map (kbd "3") (lambda () (interactive) (text-scale-test--relative-heights)))
(define-key text-scale-test-map (kbd "4") (lambda () (interactive) (text-scale-test--mixed-heights)))
(define-key text-scale-test-map (kbd "5") (lambda () (interactive) (text-scale-test--line-numbers)))
(define-key text-scale-test-map (kbd "6") (lambda () (interactive) (text-scale-test--word-wrap)))
(define-key text-scale-test-map (kbd "7") (lambda () (interactive) (text-scale-test--cursor)))
(define-key text-scale-test-map (kbd "8") (lambda () (interactive) (text-scale-test--inheritance)))
(define-key text-scale-test-map (kbd "9") (lambda () (interactive) (text-scale-test--multi-window)))
(define-key text-scale-test-map (kbd "0") (lambda () (interactive) (text-scale-test--mode-line)))

(set-transient-map text-scale-test-map t)

;; Setup on load
(message "=== Text Scale & Variable-Height Face Test Suite ===")
(message "Keys: n=next  a=run-all  1-0=specific test  c=cycle  s=stop-cycle  q=quit")
(text-scale-test--scale-levels)

;;; text-scale-test.el ends here
