;;; line-numbers-test.el --- Test line number rendering modes -*- lexical-binding: t -*-

;; Test line number display modes in the romacs GPU renderer.
;; Exercises absolute, relative, visual modes, width settings,
;; custom faces, text-scale-adjust, invisible text, and alignment.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/line-numbers-test.el

;;; Code:

(defvar line-numbers-test--step 0
  "Current test step for the automated sequence.")

(defvar line-numbers-test--timer nil
  "Timer for cycling through test modes.")

;; ---------------------------------------------------------------------------
;; Helpers
;; ---------------------------------------------------------------------------

(defun line-numbers-test--insert-numbered-lines (buf n &optional prefix)
  "Insert N lines into buffer BUF with optional PREFIX text.
Each line contains its line number and some content for readability."
  (with-current-buffer buf
    (erase-buffer)
    (dotimes (i n)
      (insert (format "%sLine %03d: %s\n"
                      (or prefix "")
                      (1+ i)
                      (if (= (mod i 5) 0)
                          "--- marker line (every 5th) ---"
                        "The quick brown fox jumps over the lazy dog."))))))

(defun line-numbers-test--section-header (title)
  "Insert a visible TITLE header at point."
  (let ((start (point)))
    (insert (format "\n=== %s ===\n\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold"))))

(defun line-numbers-test--label-window (win label)
  "Display a LABEL in the mode-line of window WIN."
  (with-selected-window win
    (setq mode-line-format
          (list " " (propertize (format " %s " label)
                                'face '(:foreground "black"
                                        :background "gold"
                                        :weight bold))
                " "
                (propertize "%b" 'face '(:foreground "cyan"))))))

;; ---------------------------------------------------------------------------
;; Test 1: Absolute line numbers (display-line-numbers = t)
;; ---------------------------------------------------------------------------

(defun line-numbers-test--absolute ()
  "Show absolute line numbers in a buffer."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*lnum-absolute*")))
    (line-numbers-test--insert-numbered-lines buf 40)
    (switch-to-buffer buf)
    (setq-local display-line-numbers t)
    (goto-char (point-min))
    (forward-line 14)
    (line-numbers-test--label-window (selected-window) "Test 1: Absolute (t)"))
  (message "Test 1: Absolute line numbers — display-line-numbers = t"))

;; ---------------------------------------------------------------------------
;; Test 2: Relative line numbers (display-line-numbers = 'relative)
;; ---------------------------------------------------------------------------

(defun line-numbers-test--relative ()
  "Show relative line numbers."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*lnum-relative*")))
    (line-numbers-test--insert-numbered-lines buf 40)
    (switch-to-buffer buf)
    (setq-local display-line-numbers 'relative)
    (goto-char (point-min))
    (forward-line 19)
    (line-numbers-test--label-window (selected-window) "Test 2: Relative"))
  (message "Test 2: Relative line numbers — current line shows 0, others show distance"))

;; ---------------------------------------------------------------------------
;; Test 3: Visual line numbers (display-line-numbers = 'visual) with word-wrap
;; ---------------------------------------------------------------------------

(defun line-numbers-test--visual ()
  "Show visual line numbers with word-wrap to demonstrate wrapped-line numbering."
  (delete-other-windows)
  (let ((buf (get-buffer-create "*lnum-visual*")))
    (with-current-buffer buf
      (erase-buffer)
      (dotimes (i 20)
        (if (= (mod i 3) 0)
            ;; Insert a very long line that will wrap
            (insert (format "Line %02d: %s %s %s\n"
                            (1+ i)
                            "This is a deliberately long line that should wrap around in the window"
                            "to demonstrate how visual line numbers handle wrapped content differently"
                            "from absolute and relative modes where only logical lines get numbers."))
          (insert (format "Line %02d: Short line.\n" (1+ i))))))
    (switch-to-buffer buf)
    (setq-local display-line-numbers 'visual)
    (setq-local word-wrap t)
    (setq truncate-lines nil)
    (goto-char (point-min))
    (forward-line 5)
    (line-numbers-test--label-window (selected-window) "Test 3: Visual + word-wrap"))
  (message "Test 3: Visual line numbers — wrapped lines get their own numbers"))

;; ---------------------------------------------------------------------------
;; Test 4: Fixed width with display-line-numbers-width
;; ---------------------------------------------------------------------------

(defun line-numbers-test--width ()
  "Show line numbers with fixed column width."
  (delete-other-windows)
  (let* ((buf-narrow (get-buffer-create "*lnum-width-narrow*"))
         (buf-wide   (get-buffer-create "*lnum-width-wide*"))
         (win-left   (selected-window))
         (win-right  (split-window-horizontally)))
    ;; Left: narrow width (2 columns)
    (line-numbers-test--insert-numbered-lines buf-narrow 30)
    (set-window-buffer win-left buf-narrow)
    (with-selected-window win-left
      (with-current-buffer buf-narrow
        (setq-local display-line-numbers t)
        (setq-local display-line-numbers-width 2))
      (goto-char (point-min)))
    (line-numbers-test--label-window win-left "Test 4a: Width=2")
    ;; Right: wide width (6 columns)
    (line-numbers-test--insert-numbered-lines buf-wide 30)
    (set-window-buffer win-right buf-wide)
    (with-selected-window win-right
      (with-current-buffer buf-wide
        (setq-local display-line-numbers t)
        (setq-local display-line-numbers-width 6))
      (goto-char (point-min)))
    (line-numbers-test--label-window win-right "Test 4b: Width=6"))
  (message "Test 4: Fixed width — left width=2, right width=6"))

;; ---------------------------------------------------------------------------
;; Test 5: display-line-numbers-current-absolute in relative mode
;; ---------------------------------------------------------------------------

(defun line-numbers-test--current-absolute ()
  "Show relative numbers with the current line displaying its absolute number."
  (delete-other-windows)
  (let* ((buf-off (get-buffer-create "*lnum-curabs-off*"))
         (buf-on  (get-buffer-create "*lnum-curabs-on*"))
         (win-left  (selected-window))
         (win-right (split-window-horizontally)))
    ;; Left: relative, current-absolute OFF
    (line-numbers-test--insert-numbered-lines buf-off 40)
    (set-window-buffer win-left buf-off)
    (with-selected-window win-left
      (with-current-buffer buf-off
        (setq-local display-line-numbers 'relative)
        (setq-local display-line-numbers-current-absolute nil))
      (goto-char (point-min))
      (forward-line 19))
    (line-numbers-test--label-window win-left "Test 5a: Relative, cur-abs=OFF")
    ;; Right: relative, current-absolute ON
    (line-numbers-test--insert-numbered-lines buf-on 40)
    (set-window-buffer win-right buf-on)
    (with-selected-window win-right
      (with-current-buffer buf-on
        (setq-local display-line-numbers 'relative)
        (setq-local display-line-numbers-current-absolute t))
      (goto-char (point-min))
      (forward-line 19))
    (line-numbers-test--label-window win-right "Test 5b: Relative, cur-abs=ON"))
  (message "Test 5: Current-absolute — left shows 0 at cursor, right shows absolute line number"))

;; ---------------------------------------------------------------------------
;; Test 6: Custom faces (line-number and line-number-current-line)
;; ---------------------------------------------------------------------------

(defun line-numbers-test--custom-faces ()
  "Apply custom faces to line-number and line-number-current-line."
  (delete-other-windows)
  (let* ((buf-default (get-buffer-create "*lnum-face-default*"))
         (buf-custom  (get-buffer-create "*lnum-face-custom*"))
         (win-left  (selected-window))
         (win-right (split-window-horizontally)))
    ;; Left: default faces
    (line-numbers-test--insert-numbered-lines buf-default 30)
    (set-window-buffer win-left buf-default)
    (with-selected-window win-left
      (with-current-buffer buf-default
        (setq-local display-line-numbers t))
      (goto-char (point-min))
      (forward-line 14))
    (line-numbers-test--label-window win-left "Test 6a: Default faces")
    ;; Right: custom faces
    (line-numbers-test--insert-numbered-lines buf-custom 30)
    (set-window-buffer win-right buf-custom)
    (with-selected-window win-right
      (with-current-buffer buf-custom
        (setq-local display-line-numbers t)
        ;; Remap line-number face: dimmed purple background
        (face-remap-add-relative 'line-number
                                 :foreground "#8888cc"
                                 :background "#1a1a2e"
                                 :weight 'light
                                 :slant 'italic)
        ;; Remap current line face: bright yellow on dark blue
        (face-remap-add-relative 'line-number-current-line
                                 :foreground "#ffdd00"
                                 :background "#003366"
                                 :weight 'bold
                                 :slant 'normal))
      (goto-char (point-min))
      (forward-line 14))
    (line-numbers-test--label-window win-right "Test 6b: Custom faces"))
  (message "Test 6: Custom faces — left=default, right=purple/yellow styled numbers"))

;; ---------------------------------------------------------------------------
;; Test 7: Line numbers with text-scale-adjust (variable font sizes)
;; ---------------------------------------------------------------------------

(defun line-numbers-test--text-scale ()
  "Show line numbers at different text scale levels."
  (delete-other-windows)
  (let* ((buf-normal  (get-buffer-create "*lnum-scale-normal*"))
         (buf-large   (get-buffer-create "*lnum-scale-large*"))
         (buf-small   (get-buffer-create "*lnum-scale-small*"))
         (win-left    (selected-window))
         (win-mid     (split-window-horizontally))
         (win-right   (split-window win-mid nil t)))
    ;; Left: normal scale (0)
    (line-numbers-test--insert-numbered-lines buf-normal 30)
    (set-window-buffer win-left buf-normal)
    (with-selected-window win-left
      (with-current-buffer buf-normal
        (setq-local display-line-numbers t)
        (text-scale-set 0))
      (goto-char (point-min)))
    (line-numbers-test--label-window win-left "Test 7a: Scale 0 (normal)")
    ;; Middle: large scale (+3)
    (line-numbers-test--insert-numbered-lines buf-large 30)
    (set-window-buffer win-mid buf-large)
    (with-selected-window win-mid
      (with-current-buffer buf-large
        (setq-local display-line-numbers t)
        (text-scale-set 3))
      (goto-char (point-min)))
    (line-numbers-test--label-window win-mid "Test 7b: Scale +3 (large)")
    ;; Right: small scale (-2)
    (line-numbers-test--insert-numbered-lines buf-small 30)
    (set-window-buffer win-right buf-small)
    (with-selected-window win-right
      (with-current-buffer buf-small
        (setq-local display-line-numbers t)
        (text-scale-set -2))
      (goto-char (point-min)))
    (line-numbers-test--label-window win-right "Test 7c: Scale -2 (small)"))
  (message "Test 7: Text-scale — normal / +3 large / -2 small, numbers should scale too"))

;; ---------------------------------------------------------------------------
;; Test 8: Line numbers with folded/invisible text
;; ---------------------------------------------------------------------------

(defun line-numbers-test--invisible ()
  "Show line numbers with invisible (folded) regions.
Uses manual `invisible' text property and outline-minor-mode headings."
  (delete-other-windows)
  (let* ((buf-invis   (get-buffer-create "*lnum-invisible*"))
         (buf-outline (get-buffer-create "*lnum-outline*"))
         (win-left    (selected-window))
         (win-right   (split-window-horizontally)))
    ;; Left: manual invisible text property
    (with-current-buffer buf-invis
      (erase-buffer)
      (dotimes (i 40)
        (let ((start (point)))
          (insert (format "Line %02d: Content for line %d\n" (1+ i) (1+ i)))
          ;; Hide lines 11-20 with invisible property
          (when (and (>= i 10) (< i 20))
            (put-text-property start (point) 'invisible t)))))
    (set-window-buffer win-left buf-invis)
    (with-selected-window win-left
      (with-current-buffer buf-invis
        (setq-local display-line-numbers t))
      (goto-char (point-min)))
    (line-numbers-test--label-window win-left "Test 8a: Lines 11-20 invisible")
    ;; Right: outline-minor-mode with folded sections
    (with-current-buffer buf-outline
      (erase-buffer)
      (insert ";;; Section 1: Introduction\n")
      (insert ";; This is the first section.\n")
      (insert ";; It has some explanatory text.\n")
      (dotimes (i 5)
        (insert (format ";; Detail line %d of section 1.\n" (1+ i))))
      (insert "\n")
      (insert ";;; Section 2: Implementation\n")
      (insert ";; Here we implement the feature.\n")
      (insert ";; More details follow.\n")
      (dotimes (i 8)
        (insert (format ";; Implementation step %d.\n" (1+ i))))
      (insert "\n")
      (insert ";;; Section 3: Testing\n")
      (insert ";; Test cases go here.\n")
      (dotimes (i 5)
        (insert (format ";; Test case %d.\n" (1+ i))))
      (insert "\n")
      (insert ";;; Section 4: Conclusion\n")
      (insert ";; Final remarks.\n")
      (insert ";; End of file.\n")
      (emacs-lisp-mode)
      (setq-local display-line-numbers t)
      (outline-minor-mode 1)
      ;; Fold sections 2 and 3
      (goto-char (point-min))
      (when (re-search-forward "^;;; Section 2" nil t)
        (outline-hide-subtree))
      (goto-char (point-min))
      (when (re-search-forward "^;;; Section 3" nil t)
        (outline-hide-subtree))
      (goto-char (point-min)))
    (set-window-buffer win-right buf-outline)
    (line-numbers-test--label-window win-right "Test 8b: Outline folded (S2,S3)"))
  (message "Test 8: Invisible text — left=lines 11-20 hidden, right=outline sections folded"))

;; ---------------------------------------------------------------------------
;; Test 9: Long file (100+ lines) alignment
;; ---------------------------------------------------------------------------

(defun line-numbers-test--long-file ()
  "Show line numbers in a 150-line buffer to verify digit alignment."
  (delete-other-windows)
  (let* ((buf-top    (get-buffer-create "*lnum-long-top*"))
         (buf-bottom (get-buffer-create "*lnum-long-bottom*"))
         (win-top    (selected-window))
         (win-bottom (split-window-vertically)))
    ;; Top: view near the beginning (single-digit to double-digit transition)
    (line-numbers-test--insert-numbered-lines buf-top 150)
    (set-window-buffer win-top buf-top)
    (with-selected-window win-top
      (with-current-buffer buf-top
        (setq-local display-line-numbers t))
      (goto-char (point-min))
      (forward-line 5))
    (line-numbers-test--label-window win-top "Test 9a: Long file (top, lines 1-...)")
    ;; Bottom: view near the end (triple-digit lines)
    (set-window-buffer win-bottom buf-bottom)
    ;; Share the same buffer but scroll differently
    (with-current-buffer buf-top
      ;; Clone content to bottom buffer
      (let ((text (buffer-string)))
        (with-current-buffer buf-bottom
          (erase-buffer)
          (insert text)
          (setq-local display-line-numbers t))))
    (with-selected-window win-bottom
      (goto-char (point-max))
      (forward-line -10))
    (line-numbers-test--label-window win-bottom "Test 9b: Long file (bottom, lines ~140-150)"))
  (message "Test 9: Long file 150 lines — top shows 1-digit/2-digit, bottom shows 3-digit alignment"))

;; ---------------------------------------------------------------------------
;; Composite: Side-by-side comparison of all three modes
;; ---------------------------------------------------------------------------

(defun line-numbers-test--side-by-side ()
  "Show absolute, relative, and visual modes side by side."
  (delete-other-windows)
  (let* ((buf-abs  (get-buffer-create "*lnum-cmp-absolute*"))
         (buf-rel  (get-buffer-create "*lnum-cmp-relative*"))
         (buf-vis  (get-buffer-create "*lnum-cmp-visual*"))
         (win-left   (selected-window))
         (win-mid    (split-window-horizontally))
         (win-right  (split-window win-mid nil t)))
    ;; Populate all three buffers with the same content (some long lines for visual)
    (dolist (buf (list buf-abs buf-rel buf-vis))
      (with-current-buffer buf
        (erase-buffer)
        (dotimes (i 30)
          (if (= (mod i 4) 0)
              (insert (format "Line %02d: A long line that should wrap in visual mode to show difference %s\n"
                              (1+ i)
                              (make-string 40 ?=)))
            (insert (format "Line %02d: Normal content.\n" (1+ i)))))))
    ;; Left: absolute
    (set-window-buffer win-left buf-abs)
    (with-selected-window win-left
      (with-current-buffer buf-abs
        (setq-local display-line-numbers t)
        (setq truncate-lines nil)
        (setq-local word-wrap t))
      (goto-char (point-min))
      (forward-line 14))
    (line-numbers-test--label-window win-left "Absolute (t)")
    ;; Middle: relative
    (set-window-buffer win-mid buf-rel)
    (with-selected-window win-mid
      (with-current-buffer buf-rel
        (setq-local display-line-numbers 'relative)
        (setq truncate-lines nil)
        (setq-local word-wrap t))
      (goto-char (point-min))
      (forward-line 14))
    (line-numbers-test--label-window win-mid "Relative")
    ;; Right: visual
    (set-window-buffer win-right buf-vis)
    (with-selected-window win-right
      (with-current-buffer buf-vis
        (setq-local display-line-numbers 'visual)
        (setq truncate-lines nil)
        (setq-local word-wrap t))
      (goto-char (point-min))
      (forward-line 14))
    (line-numbers-test--label-window win-right "Visual"))
  (message "Side-by-side: absolute | relative | visual — compare numbering with wrapped lines"))

;; ============================================================================
;; Test runner
;; ============================================================================

(defvar line-numbers-test--tests
  '(("Absolute line numbers"      . line-numbers-test--absolute)
    ("Relative line numbers"      . line-numbers-test--relative)
    ("Visual + word-wrap"         . line-numbers-test--visual)
    ("Fixed width"                . line-numbers-test--width)
    ("Current-absolute"           . line-numbers-test--current-absolute)
    ("Custom faces"               . line-numbers-test--custom-faces)
    ("Text-scale-adjust"          . line-numbers-test--text-scale)
    ("Invisible/folded text"      . line-numbers-test--invisible)
    ("Long file alignment"        . line-numbers-test--long-file)
    ("Side-by-side comparison"    . line-numbers-test--side-by-side))
  "Alist of (name . function) for line number tests.")

(defun line-numbers-test-next ()
  "Run the next test in the sequence."
  (interactive)
  (when (>= line-numbers-test--step (length line-numbers-test--tests))
    (setq line-numbers-test--step 0))
  (let ((test (nth line-numbers-test--step line-numbers-test--tests)))
    (message "--- Running test %d/%d: %s ---"
             (1+ line-numbers-test--step)
             (length line-numbers-test--tests)
             (car test))
    (funcall (cdr test))
    (setq line-numbers-test--step (1+ line-numbers-test--step))))

(defun line-numbers-test-run-all ()
  "Run all tests sequentially with pauses for observation."
  (interactive)
  (setq line-numbers-test--step 0)
  (when line-numbers-test--timer
    (cancel-timer line-numbers-test--timer)
    (setq line-numbers-test--timer nil))
  (let ((i 0))
    (dolist (test line-numbers-test--tests)
      (run-at-time (* i 4) nil
                   (lambda (test-pair idx)
                     (message "\n=== Test %d/%d: %s ==="
                              (1+ idx)
                              (length line-numbers-test--tests)
                              (car test-pair))
                     (funcall (cdr test-pair)))
                   test i)
      (setq i (1+ i)))
    ;; Final message
    (run-at-time (* i 4) nil
                 (lambda ()
                   (message "\n=== All %d line number tests complete ==="
                            (length line-numbers-test--tests))
                   (message "Keys: n=next  a=run-all  0=side-by-side  1-9=specific test  q=cleanup"))))
  (message "Running all %d tests (4 second intervals)..." (length line-numbers-test--tests)))

(defun line-numbers-test--cleanup ()
  "Kill all test buffers."
  (dolist (name '("*lnum-absolute*" "*lnum-relative*" "*lnum-visual*"
                  "*lnum-width-narrow*" "*lnum-width-wide*"
                  "*lnum-curabs-off*" "*lnum-curabs-on*"
                  "*lnum-face-default*" "*lnum-face-custom*"
                  "*lnum-scale-normal*" "*lnum-scale-large*" "*lnum-scale-small*"
                  "*lnum-invisible*" "*lnum-outline*"
                  "*lnum-long-top*" "*lnum-long-bottom*"
                  "*lnum-cmp-absolute*" "*lnum-cmp-relative*" "*lnum-cmp-visual*"))
    (when (get-buffer name)
      (kill-buffer name)))
  (when line-numbers-test--timer
    (cancel-timer line-numbers-test--timer)
    (setq line-numbers-test--timer nil))
  (delete-other-windows)
  (message "All line-number test buffers cleaned up."))

;; Keybindings for interactive use
(defvar line-numbers-test-map (make-sparse-keymap)
  "Keymap for line number test.")

(define-key line-numbers-test-map (kbd "n") #'line-numbers-test-next)
(define-key line-numbers-test-map (kbd "a") #'line-numbers-test-run-all)
(define-key line-numbers-test-map (kbd "q") (lambda () (interactive) (line-numbers-test--cleanup)))
(define-key line-numbers-test-map (kbd "1") (lambda () (interactive) (line-numbers-test--absolute)))
(define-key line-numbers-test-map (kbd "2") (lambda () (interactive) (line-numbers-test--relative)))
(define-key line-numbers-test-map (kbd "3") (lambda () (interactive) (line-numbers-test--visual)))
(define-key line-numbers-test-map (kbd "4") (lambda () (interactive) (line-numbers-test--width)))
(define-key line-numbers-test-map (kbd "5") (lambda () (interactive) (line-numbers-test--current-absolute)))
(define-key line-numbers-test-map (kbd "6") (lambda () (interactive) (line-numbers-test--custom-faces)))
(define-key line-numbers-test-map (kbd "7") (lambda () (interactive) (line-numbers-test--text-scale)))
(define-key line-numbers-test-map (kbd "8") (lambda () (interactive) (line-numbers-test--invisible)))
(define-key line-numbers-test-map (kbd "9") (lambda () (interactive) (line-numbers-test--long-file)))
(define-key line-numbers-test-map (kbd "0") (lambda () (interactive) (line-numbers-test--side-by-side)))

(set-transient-map line-numbers-test-map t)

;; Setup on load
(message "=== Line Numbers Test Suite ===")
(message "Keys: n=next  a=run-all  1-9=specific test  0=side-by-side  q=cleanup")
(line-numbers-test--absolute)

;;; line-numbers-test.el ends here
