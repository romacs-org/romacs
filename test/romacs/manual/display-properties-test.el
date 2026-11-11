;;; display-properties-test.el --- Test display property rendering -*- lexical-binding: t -*-

;; Test Emacs display properties rendering in the romacs GPU renderer.
;; Covers: display strings, space specs, alignment, raise, height scaling,
;; before/after-string overlays, overlay faces, invisible text, images,
;; and combinations of the above.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/display-properties-test.el

;;; Code:

(defvar display-prop-test--overlays nil
  "List of overlays created during the test, for cleanup.")

(defun display-prop-test--cleanup ()
  "Remove all test overlays."
  (dolist (ov display-prop-test--overlays)
    (when (overlay-buffer ov)
      (delete-overlay ov)))
  (setq display-prop-test--overlays nil))

(defun display-prop-test--insert-section (title)
  "Insert a section header with TITLE."
  (insert "\n")
  (let ((start (point)))
    (insert (format "=== %s ===\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold"))))

(defun display-prop-test--insert-description (text)
  "Insert a description TEXT in a dimmed face."
  (let ((start (point)))
    (insert (format "  [%s]\n" text))
    (put-text-property start (point) 'face '(:foreground "gray60" :slant italic))))

(defun display-prop-test--make-overlay (beg end &rest properties)
  "Create an overlay from BEG to END with PROPERTIES, tracking it for cleanup."
  (let ((ov (make-overlay beg end)))
    (while properties
      (overlay-put ov (pop properties) (pop properties)))
    (push ov display-prop-test--overlays)
    ov))

(defun display-prop-test ()
  "Create buffer with display property test cases."
  (interactive)
  (display-prop-test--cleanup)
  (let ((buf (get-buffer-create "*Display Properties Test*")))
    (switch-to-buffer buf)
    (setq buffer-read-only nil)
    (erase-buffer)

    ;; Title
    (let ((start (point)))
      (insert "DISPLAY PROPERTIES RENDERING TEST\n")
      (put-text-property start (point) 'face '(:weight bold :height 1.8 :foreground "cyan")))
    (insert (format "Window system: %s\n" window-system))
    (insert (make-string 72 ?-) "\n")

    ;; ========================================================================
    ;; 1. Display property: string replacement
    ;; ========================================================================
    (display-prop-test--insert-section "1. DISPLAY STRING REPLACEMENT")
    (display-prop-test--insert-description
     "The underlying text is replaced visually by a different string")

    (insert "  Original text: ")
    (let ((start (point)))
      (insert "THIS TEXT IS HIDDEN")
      (put-text-property start (point) 'display
                         (propertize "REPLACEMENT STRING"
                                     'face '(:foreground "lime green" :weight bold))))
    (insert "\n")

    (insert "  Another replacement: ")
    (let ((start (point)))
      (insert "invisible-original")
      (put-text-property start (point) 'display
                         (propertize "[ICON]" 'face '(:foreground "orange" :weight bold))))
    (insert " <-- should show [ICON] not invisible-original\n")

    (insert "  Multi-line replacement: ")
    (let ((start (point)))
      (insert "XXXXXXXXXX")
      (put-text-property start (point) 'display
                         (propertize "Line1\n                             Line2"
                                     'face '(:foreground "sky blue"))))
    (insert "\n")

    ;; ========================================================================
    ;; 2. Display property: (space :width N)
    ;; ========================================================================
    (display-prop-test--insert-section "2. SPACE :WIDTH — FIXED-WIDTH SPACES")
    (display-prop-test--insert-description
     "Markers | should be spaced apart by specified pixel/column widths")

    (insert "  |")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :width 5)))
    (insert "| <-- 5 columns apart\n")

    (insert "  |")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :width 10)))
    (insert "| <-- 10 columns apart\n")

    (insert "  |")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :width 20)))
    (insert "| <-- 20 columns apart\n")

    (insert "  |")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :width 40)))
    (insert "| <-- 40 columns apart\n")

    ;; ========================================================================
    ;; 3. Display property: (space :align-to N)
    ;; ========================================================================
    (display-prop-test--insert-section "3. SPACE :ALIGN-TO — COLUMN ALIGNMENT")
    (display-prop-test--insert-description
     "The word ALIGNED should start at the specified column number")

    (insert "  Col 20:")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :align-to 20)))
    (insert (propertize "ALIGNED" 'face '(:foreground "yellow")))
    (insert "\n")

    (insert "  Col 30:")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :align-to 30)))
    (insert (propertize "ALIGNED" 'face '(:foreground "yellow")))
    (insert "\n")

    (insert "  Col 40:")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :align-to 40)))
    (insert (propertize "ALIGNED" 'face '(:foreground "yellow")))
    (insert "\n")

    (insert "  Col 50:")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :align-to 50)))
    (insert (propertize "ALIGNED" 'face '(:foreground "yellow")))
    (insert "\n")

    ;; Ruler line
    (insert "  ")
    (let ((start (point)))
      (insert "0         1         2         3         4         5         6\n")
      (put-text-property start (point) 'face '(:foreground "gray50")))
    (insert "  ")
    (let ((start (point)))
      (insert "0123456789012345678901234567890123456789012345678901234567890\n")
      (put-text-property start (point) 'face '(:foreground "gray50")))

    ;; ========================================================================
    ;; 4. Display property: (raise N)
    ;; ========================================================================
    (display-prop-test--insert-section "4. RAISE — SUPERSCRIPT / SUBSCRIPT EFFECT")
    (display-prop-test--insert-description
     "Text should be raised or lowered relative to the baseline")

    (insert "  Normal")
    (let ((start (point)))
      (insert "raised+0.3")
      (put-text-property start (point) 'display '(raise 0.3))
      (put-text-property start (point) 'face '(:foreground "light green")))
    (insert "Normal")
    (let ((start (point)))
      (insert "raised+0.5")
      (put-text-property start (point) 'display '(raise 0.5))
      (put-text-property start (point) 'face '(:foreground "light blue")))
    (insert "Normal\n")

    (insert "  Normal")
    (let ((start (point)))
      (insert "lowered-0.3")
      (put-text-property start (point) 'display '(raise -0.3))
      (put-text-property start (point) 'face '(:foreground "salmon")))
    (insert "Normal")
    (let ((start (point)))
      (insert "lowered-0.5")
      (put-text-property start (point) 'display '(raise -0.5))
      (put-text-property start (point) 'face '(:foreground "plum")))
    (insert "Normal\n")

    (insert "  Use case: E=mc")
    (let ((start (point)))
      (insert "2")
      (put-text-property start (point) 'display '(raise 0.3))
      (put-text-property start (point) 'face '(:height 0.8 :foreground "cyan")))
    (insert "   H")
    (let ((start (point)))
      (insert "2")
      (put-text-property start (point) 'display '(raise -0.2))
      (put-text-property start (point) 'face '(:height 0.8 :foreground "cyan")))
    (insert "O\n")

    ;; ========================================================================
    ;; 5. Display property: (height N)
    ;; ========================================================================
    (display-prop-test--insert-section "5. HEIGHT — TEXT SCALING")
    (display-prop-test--insert-description
     "Text should appear at different sizes relative to default")

    (insert "  ")
    (let ((start (point)))
      (insert "height 0.6")
      (put-text-property start (point) 'face '(:height 0.6 :foreground "gray70")))
    (insert "  ")
    (let ((start (point)))
      (insert "height 0.8")
      (put-text-property start (point) 'face '(:height 0.8 :foreground "gray80")))
    (insert "  ")
    (let ((start (point)))
      (insert "height 1.0 (default)")
      (put-text-property start (point) 'face '(:foreground "white")))
    (insert "\n  ")
    (let ((start (point)))
      (insert "height 1.3")
      (put-text-property start (point) 'face '(:height 1.3 :foreground "light green")))
    (insert "  ")
    (let ((start (point)))
      (insert "height 1.6")
      (put-text-property start (point) 'face '(:height 1.6 :foreground "light blue")))
    (insert "  ")
    (let ((start (point)))
      (insert "height 2.0")
      (put-text-property start (point) 'face '(:height 2.0 :foreground "gold")))
    (insert "\n")

    ;; ========================================================================
    ;; 6. Before-string and after-string overlays
    ;; ========================================================================
    (display-prop-test--insert-section "6. BEFORE-STRING & AFTER-STRING OVERLAYS")
    (display-prop-test--insert-description
     "Extra strings injected before/after text via overlays")

    (insert "  ")
    (let ((line-start (point)))
      (insert "This line has a before-string overlay")
      (display-prop-test--make-overlay
       line-start (1+ line-start)
       'before-string (propertize ">>> " 'face '(:foreground "orange" :weight bold))))
    (insert "\n")

    (insert "  ")
    (let ((line-start (point)))
      (insert "This line has an after-string overlay")
      (let ((line-end (point)))
        (display-prop-test--make-overlay
         (1- line-end) line-end
         'after-string (propertize " <<<" 'face '(:foreground "orange" :weight bold)))))
    (insert "\n")

    (insert "  ")
    (let ((line-start (point)))
      (insert "Both before and after")
      (let ((line-end (point)))
        (display-prop-test--make-overlay
         line-start (1+ line-start)
         'before-string (propertize "[START] " 'face '(:foreground "cyan" :weight bold)))
        (display-prop-test--make-overlay
         (1- line-end) line-end
         'after-string (propertize " [END]" 'face '(:foreground "magenta" :weight bold)))))
    (insert "\n")

    (insert "  ")
    (let ((line-start (point)))
      (insert "Line number prefix")
      (display-prop-test--make-overlay
       line-start (1+ line-start)
       'before-string (propertize "  42 | " 'face '(:foreground "gray50"))))
    (insert "\n")

    ;; ========================================================================
    ;; 7. Overlay face attributes
    ;; ========================================================================
    (display-prop-test--insert-section "7. OVERLAY FACE — HIGHLIGHTED REGIONS")
    (display-prop-test--insert-description
     "Regions highlighted with overlay face properties")

    (insert "  This sentence has ")
    (let ((hl-start (point)))
      (insert "yellow highlighted words")
      (display-prop-test--make-overlay
       hl-start (point)
       'face '(:background "dark goldenrod" :foreground "white")))
    (insert " in the middle.\n")

    (insert "  This has a ")
    (let ((hl-start (point)))
      (insert "red background region")
      (display-prop-test--make-overlay
       hl-start (point)
       'face '(:background "dark red" :foreground "white" :weight bold)))
    (insert " and normal text after.\n")

    (insert "  Overlapping overlays: ")
    (let ((start1 (point)))
      (insert "AABBCCDD")
      (let ((mid (+ start1 4)))
        ;; First overlay covers AABB
        (display-prop-test--make-overlay
         start1 mid
         'face '(:background "dark blue" :foreground "white"))
        ;; Second overlay covers CCDD
        (display-prop-test--make-overlay
         mid (point)
         'face '(:background "dark green" :foreground "white"))))
    (insert " <-- blue then green\n")

    (insert "  Underline overlay: ")
    (let ((start (point)))
      (insert "this text has a wave underline via overlay")
      (display-prop-test--make-overlay
       start (point)
       'face '(:underline (:style wave :color "red"))))
    (insert "\n")

    ;; ========================================================================
    ;; 8. Invisible text
    ;; ========================================================================
    (display-prop-test--insert-section "8. INVISIBLE TEXT — HIDDEN WITH/WITHOUT ELLIPSIS")
    (display-prop-test--insert-description
     "Some text is invisible; some shows ... ellipsis")

    (insert "  Visible[")
    (let ((start (point)))
      (insert "THIS IS INVISIBLE")
      (put-text-property start (point) 'invisible t))
    (insert "]Visible <-- nothing between brackets\n")

    ;; Invisible with ellipsis using GNU's per-buffer invisibility spec.
    (setq-local buffer-invisibility-spec
                '(t (display-test-ellipsis . t) display-test-no-ellipsis))

    (insert "  Visible[")
    (let ((start (point)))
      (insert "HIDDEN BUT WITH ELLIPSIS")
      (put-text-property start (point) 'invisible 'display-test-ellipsis))
    (insert "]Visible <-- should show [...] between brackets\n")

    ;; Invisible without ellipsis

    (insert "  Start ")
    (let ((start (point)))
      (insert "XXXXXX")
      (put-text-property start (point) 'invisible 'display-test-no-ellipsis))
    (insert " End <-- nothing between Start and End\n")

    (insert "  A")
    (let ((start (point)))
      (insert "hidden1")
      (put-text-property start (point) 'invisible t))
    (insert "B")
    (let ((start (point)))
      (insert "hidden2")
      (put-text-property start (point) 'invisible t))
    (insert "C <-- should read ABC with nothing else\n")

    ;; ========================================================================
    ;; 9. Display property: (image ...)
    ;; ========================================================================
    (display-prop-test--insert-section "9. IMAGE DISPLAY PROPERTY (PLACEHOLDER)")
    (display-prop-test--insert-description
     "Image display property with a 1x1 XPM placeholder since real images may not exist")

    ;; Create a simple XPM image inline (guaranteed to work without external files)
    (let ((xpm-red "/* XPM */\nstatic char *r[] = {\n\"16 16 2 1\",\n\"  c None\",\n\"# c #FF4444\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\"};")
          (xpm-green "/* XPM */\nstatic char *g[] = {\n\"16 16 2 1\",\n\"  c None\",\n\"# c #44FF44\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\"};")
          (xpm-blue "/* XPM */\nstatic char *b[] = {\n\"16 16 2 1\",\n\"  c None\",\n\"# c #4444FF\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\",\n\"################\"};"))

      (insert "  Inline XPM images: ")
      (let ((start (point)))
        (insert "R")
        (put-text-property start (point) 'display (create-image xpm-red 'xpm t)))
      (insert " ")
      (let ((start (point)))
        (insert "G")
        (put-text-property start (point) 'display (create-image xpm-green 'xpm t)))
      (insert " ")
      (let ((start (point)))
        (insert "B")
        (put-text-property start (point) 'display (create-image xpm-blue 'xpm t)))
      (insert " <-- red, green, blue 16x16 squares\n"))

    (insert "  Image with text: before ")
    (let ((start (point)))
      (insert "X")
      (put-text-property start (point)
                         'display (create-image
                                   "/* XPM */\nstatic char *x[] = {\n\"8 8 2 1\",\n\"  c None\",\n\"# c #FFAA00\",\n\"########\",\n\"#      #\",\n\"# #### #\",\n\"# #  # #\",\n\"# #  # #\",\n\"# #### #\",\n\"#      #\",\n\"########\"};"
                                   'xpm t)))
    (insert " after <-- orange 8x8 box between words\n")

    ;; ========================================================================
    ;; 10. Combinations
    ;; ========================================================================
    (display-prop-test--insert-section "10. COMBINATIONS — MIXED DISPLAY PROPERTIES")
    (display-prop-test--insert-description
     "Multiple display properties used together")

    ;; Invisible text followed by visible
    (insert "  ")
    (let ((start (point)))
      (insert "SECRET")
      (put-text-property start (point) 'invisible t))
    (let ((start (point)))
      (insert "Visible after invisible")
      (put-text-property start (point) 'face '(:foreground "light green")))
    (insert " <-- should only show green text\n")

    ;; Overlay face + display string replacement
    (insert "  ")
    (let ((start (point)))
      (insert "original text here")
      (let ((end (point)))
        (put-text-property start end 'display
                           (propertize "DISPLAY REPLACEMENT" 'face '(:foreground "white")))
        (display-prop-test--make-overlay
         start end
         'face '(:background "dark magenta"))))
    (insert " <-- replaced text with magenta background overlay\n")

    ;; Before-string with raised text
    (insert "  ")
    (let ((start (point)))
      (insert "Main text with prefix")
      (display-prop-test--make-overlay
       start (1+ start)
       'before-string (propertize "^note " 'face '(:foreground "yellow" :height 0.7)
                                  'display '(raise 0.3))))
    (insert "\n")

    ;; Mixed height text on one line
    (insert "  ")
    (let ((start (point)))
      (insert "BIG")
      (put-text-property start (point) 'face '(:height 2.0 :foreground "gold" :weight bold)))
    (insert " and ")
    (let ((start (point)))
      (insert "small")
      (put-text-property start (point) 'face '(:height 0.7 :foreground "gray70")))
    (insert " and ")
    (let ((start (point)))
      (insert "normal")
      (put-text-property start (point) 'face '(:foreground "white")))
    (insert " on one line\n")

    ;; Space alignment with face
    (insert "  Name:")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :align-to 20)))
    (let ((start (point)))
      (insert "Alice")
      (put-text-property start (point) 'face '(:foreground "cyan")))
    (insert "\n")
    (insert "  Score:")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :align-to 20)))
    (let ((start (point)))
      (insert "9001")
      (put-text-property start (point) 'face '(:foreground "lime green" :weight bold)))
    (insert "\n")
    (insert "  Rank:")
    (let ((start (point)))
      (insert " ")
      (put-text-property start (point) 'display '(space :align-to 20)))
    (let ((start (point)))
      (insert "#1")
      (put-text-property start (point) 'face '(:foreground "gold" :weight bold)))
    (insert "\n")

    ;; Display string inside an overlay
    (insert "  Overlay display: ")
    (let ((start (point)))
      (insert "HIDDEN BY OVERLAY DISPLAY")
      (display-prop-test--make-overlay
       start (point)
       'display (propertize "OVERLAY REPLACED THIS"
                            'face '(:foreground "orchid" :weight bold))))
    (insert "\n")

    ;; ========================================================================
    ;; Summary
    ;; ========================================================================
    (insert "\n")
    (let ((start (point)))
      (insert (make-string 72 ?-) "\n")
      (put-text-property start (point) 'face '(:foreground "gray40")))
    (let ((start (point)))
      (insert "END OF DISPLAY PROPERTIES TEST\n")
      (put-text-property start (point) 'face '(:weight bold :foreground "cyan")))
    (insert "If all sections above render correctly, the romacs GPU renderer\n")
    (insert "handles display properties properly.\n")

    (goto-char (point-min))
    (setq buffer-read-only t)
    (message "Display properties test ready. Inspect each section visually.")))

(display-prop-test)

;;; display-properties-test.el ends here
