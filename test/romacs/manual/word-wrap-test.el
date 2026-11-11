;;; word-wrap-test.el --- Test word wrap and truncation rendering -*- lexical-binding: t -*-

;; Test word wrap, line truncation, and horizontal scrolling in the
;; romacs GPU renderer.  Creates multiple windows showing different
;; wrapping modes simultaneously.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/word-wrap-test.el

;;; Code:

;; ---------------------------------------------------------------------------
;; Helpers
;; ---------------------------------------------------------------------------

(defun word-wrap-test--heading (text)
  "Insert a highlighted section heading TEXT."
  (let ((start (point)))
    (insert (format "=== %s ===\n" text))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold"))))

(defun word-wrap-test--label (text)
  "Insert a dim label TEXT."
  (let ((start (point)))
    (insert text)
    (put-text-property start (point) 'face '(:foreground "gray60"))))

(defun word-wrap-test--long-latin (&optional n)
  "Return a long Latin sentence repeated to exceed N (default 220) chars."
  (let ((base "The quick brown fox jumps over the lazy dog. "))
    (let ((s ""))
      (while (< (length s) (or n 220))
        (setq s (concat s base)))
      s)))

(defun word-wrap-test--long-word (&optional n)
  "Return a single word (no spaces) of length N (default 250)."
  (let ((chars "abcdefghijklmnopqrstuvwxyz"))
    (let ((s ""))
      (while (< (length s) (or n 250))
        (setq s (concat s chars)))
      (substring s 0 (or n 250)))))

(defun word-wrap-test--mixed-cjk-line ()
  "Return a line mixing Latin, CJK, tabs, and spaces."
  (concat
   "Hello\t"
   "\u4f60\u597d\u4e16\u754c"       ;; Chinese: hello world
   "  mixed  "
   "\u3053\u3093\u306b\u3061\u306f" ;; Japanese: konnichiwa
   "\ttab\there\t"
   "\uc548\ub155\ud558\uc138\uc694" ;; Korean: hello
   " END-OF-LINE "
   "The quick brown fox jumps over the lazy dog. "
   "\u4e16\u754c\u4f60\u597d "
   "\u3053\u3093\u306b\u3061\u306f "
   "Lorem ipsum dolor sit amet, consectetur adipiscing elit."))

(defun word-wrap-test--faced-long-line ()
  "Return a propertized long line with face changes every few words."
  (let ((words '("alpha" "bravo" "charlie" "delta" "echo"
                 "foxtrot" "golf" "hotel" "india" "juliet"
                 "kilo" "lima" "mike" "november" "oscar"
                 "papa" "quebec" "romeo" "sierra" "tango"
                 "uniform" "victor" "whiskey" "xray" "yankee" "zulu"))
        (faces `((:foreground "coral")
                 (:foreground "cyan" :weight bold)
                 (:foreground "lime green" :slant italic)
                 (:foreground "orchid" :underline t)
                 (:foreground "gold" :weight bold)
                 (:foreground "sky blue" :box (:line-width 1 :color "sky blue"))
                 (:background "dark red" :foreground "white")
                 (:foreground "spring green")))
        (result ""))
    (dotimes (i 40)
      (let* ((w (nth (mod i (length words)) words))
             (f (nth (mod i (length faces)) faces))
             (chunk (propertize (concat w " ") 'face f)))
        (setq result (concat result chunk))))
    result))

;; ---------------------------------------------------------------------------
;; Buffer builders
;; ---------------------------------------------------------------------------

(defun word-wrap-test--buf-truncate ()
  "Create buffer demonstrating `truncate-lines' t."
  (let ((buf (get-buffer-create "*wrap: truncate-lines*")))
    (with-current-buffer buf
      (erase-buffer)
      (word-wrap-test--heading "truncate-lines = t")
      (word-wrap-test--label
       "Long lines are clipped at the right edge with a $ fringe indicator.\n\n")
      (insert (word-wrap-test--long-latin 250) "\n")
      (insert (word-wrap-test--long-latin 300) "\n")
      (insert "Short line.\n")
      (insert (word-wrap-test--long-latin 200) "\n")
      (insert "\n")
      (word-wrap-test--label "--- very long word (no spaces) ---\n")
      (insert (word-wrap-test--long-word 260) "\n")
      (insert "\n")
      (word-wrap-test--label "--- mixed CJK / tabs ---\n")
      (insert (word-wrap-test--mixed-cjk-line) "\n")
      (insert (word-wrap-test--mixed-cjk-line) "\n")
      (insert "\n")
      (word-wrap-test--label "--- face changes mid-line ---\n")
      (insert (word-wrap-test--faced-long-line) "\n")
      (insert (word-wrap-test--faced-long-line) "\n")
      (goto-char (point-min))
      (setq truncate-lines t)
      (setq-local word-wrap nil))
    buf))

(defun word-wrap-test--buf-char-wrap ()
  "Create buffer demonstrating character-level wrapping (continuation \\)."
  (let ((buf (get-buffer-create "*wrap: char-wrap*")))
    (with-current-buffer buf
      (erase-buffer)
      (word-wrap-test--heading "truncate-lines=nil, word-wrap=nil (char wrap)")
      (word-wrap-test--label
       "Lines wrap at the window edge at any character.  Continuation \\ indicator.\n\n")
      (insert (word-wrap-test--long-latin 250) "\n")
      (insert (word-wrap-test--long-latin 300) "\n")
      (insert "Short line.\n")
      (insert "\n")
      (word-wrap-test--label "--- very long word (no spaces) ---\n")
      (insert (word-wrap-test--long-word 260) "\n")
      (insert "\n")
      (word-wrap-test--label "--- mixed CJK / tabs ---\n")
      (insert (word-wrap-test--mixed-cjk-line) "\n")
      (insert (word-wrap-test--mixed-cjk-line) "\n")
      (insert "\n")
      (word-wrap-test--label "--- face changes mid-line ---\n")
      (insert (word-wrap-test--faced-long-line) "\n")
      (insert (word-wrap-test--faced-long-line) "\n")
      (goto-char (point-min))
      (setq truncate-lines nil)
      (setq-local word-wrap nil))
    buf))

(defun word-wrap-test--buf-word-wrap ()
  "Create buffer demonstrating `word-wrap' t (break at spaces)."
  (let ((buf (get-buffer-create "*wrap: word-wrap*")))
    (with-current-buffer buf
      (erase-buffer)
      (word-wrap-test--heading "word-wrap = t (break at whitespace)")
      (word-wrap-test--label
       "Lines break at word boundaries.  No continuation indicator.\n\n")
      (insert (word-wrap-test--long-latin 250) "\n")
      (insert (word-wrap-test--long-latin 300) "\n")
      (insert "Short line that fits in the window.\n")
      (insert "\n")
      (word-wrap-test--label "--- very long word (no spaces) ---\n")
      (word-wrap-test--label "Should wrap inside the word since there is no break point:\n")
      (insert (word-wrap-test--long-word 260) "\n")
      (insert "\n")
      (word-wrap-test--label "--- mixed CJK / tabs ---\n")
      (insert (word-wrap-test--mixed-cjk-line) "\n")
      (insert (word-wrap-test--mixed-cjk-line) "\n")
      (insert "\n")
      (word-wrap-test--label "--- face changes mid-line ---\n")
      (insert (word-wrap-test--faced-long-line) "\n")
      (insert (word-wrap-test--faced-long-line) "\n")
      (goto-char (point-min))
      (setq truncate-lines nil)
      (setq-local word-wrap t))
    buf))

(defun word-wrap-test--buf-visual-line ()
  "Create buffer with `visual-line-mode' enabled."
  (let ((buf (get-buffer-create "*wrap: visual-line-mode*")))
    (with-current-buffer buf
      (erase-buffer)
      (word-wrap-test--heading "visual-line-mode")
      (word-wrap-test--label
       "word-wrap + visual-line fringe indicators + movement by visual line.\n\n")
      (insert (word-wrap-test--long-latin 250) "\n")
      (insert (word-wrap-test--long-latin 300) "\n")
      (insert "Short line.\n")
      (insert "\n")
      (word-wrap-test--label "--- very long word (no spaces) ---\n")
      (insert (word-wrap-test--long-word 260) "\n")
      (insert "\n")
      (word-wrap-test--label "--- mixed CJK / tabs ---\n")
      (insert (word-wrap-test--mixed-cjk-line) "\n")
      (insert (word-wrap-test--mixed-cjk-line) "\n")
      (insert "\n")
      (word-wrap-test--label "--- face changes mid-line ---\n")
      (insert (word-wrap-test--faced-long-line) "\n")
      (insert (word-wrap-test--faced-long-line) "\n")
      (goto-char (point-min))
      (visual-line-mode 1))
    buf))

;; ---------------------------------------------------------------------------
;; Test 7: fill-column-indicator-mode
;; ---------------------------------------------------------------------------

(defun word-wrap-test--buf-fill-column ()
  "Create buffer with `display-fill-column-indicator-mode'."
  (let ((buf (get-buffer-create "*wrap: fill-column-indicator*")))
    (with-current-buffer buf
      (erase-buffer)
      (word-wrap-test--heading "fill-column-indicator-mode (fill-column=80)")
      (word-wrap-test--label
       "Vertical line at column 80.  Word-wrap enabled so text wraps at spaces.\n\n")
      (setq-local fill-column 80)
      (insert (word-wrap-test--long-latin 250) "\n")
      (insert (word-wrap-test--long-latin 300) "\n")
      (insert "Short line.\n")
      (dotimes (i 8)
        (insert (format "Line %d: %s\n" (1+ i) (word-wrap-test--long-latin 150))))
      (goto-char (point-min))
      (setq truncate-lines nil)
      (setq-local word-wrap t)
      (display-fill-column-indicator-mode 1))
    buf))

;; ---------------------------------------------------------------------------
;; Test 8: truncate-partial-width-windows
;; ---------------------------------------------------------------------------

(defun word-wrap-test--buf-partial-width ()
  "Create buffer for testing `truncate-partial-width-windows' in splits."
  (let ((buf (get-buffer-create "*wrap: partial-width*")))
    (with-current-buffer buf
      (erase-buffer)
      (word-wrap-test--heading "truncate-partial-width-windows")
      (word-wrap-test--label
       (concat "When truncate-partial-width-windows is non-nil (default),\n"
               "split windows narrower than the threshold auto-truncate lines.\n"
               "Resize windows to see behavior change.\n\n"))
      (insert (word-wrap-test--long-latin 250) "\n")
      (insert (word-wrap-test--long-latin 300) "\n")
      (insert "Short line.\n")
      (dotimes (i 6)
        (insert (format "Content line %d: %s\n" (1+ i) (word-wrap-test--long-latin 180))))
      (goto-char (point-min))
      ;; Default: truncate in partial-width windows
      (setq truncate-lines nil)
      (setq-local word-wrap t)
      (setq truncate-partial-width-windows t))
    buf))

;; ---------------------------------------------------------------------------
;; Test 10: horizontal scrolling
;; ---------------------------------------------------------------------------

(defun word-wrap-test--buf-hscroll ()
  "Create buffer for testing horizontal scrolling."
  (let ((buf (get-buffer-create "*wrap: hscroll*")))
    (with-current-buffer buf
      (erase-buffer)
      (word-wrap-test--heading "Horizontal scrolling (truncate + hscroll)")
      (word-wrap-test--label
       (concat "truncate-lines=t, auto-hscroll-mode=t, hscroll-margin=5.\n"
               "Move cursor into a long line to trigger horizontal scrolling.\n"
               "Use C-x < and C-x > to scroll manually.\n\n"))
      ;; Insert numbered long lines so scroll offset is visible
      (dotimes (i 20)
        (let ((prefix (format "[line-%03d] " (1+ i))))
          (insert prefix)
          (cond
           ;; Even lines: long Latin
           ((= (mod i 3) 0)
            (insert (word-wrap-test--long-latin 300)))
           ;; Odd lines: faced content
           ((= (mod i 3) 1)
            (insert (word-wrap-test--faced-long-line)))
           ;; Every third: mixed CJK
           (t
            (insert (word-wrap-test--mixed-cjk-line))))
          (insert "\n")))
      (insert "\n")
      (word-wrap-test--label "--- end of content ---\n")
      (goto-char (point-min))
      (setq truncate-lines t)
      (setq-local auto-hscroll-mode t)
      (setq-local hscroll-margin 5))
    buf))

;; ---------------------------------------------------------------------------
;; Window layout
;; ---------------------------------------------------------------------------

(defun word-wrap-test--setup-windows ()
  "Arrange 4 windows showing different wrapping modes simultaneously.

Layout (approximately):

  +---------------------+---------------------+
  | truncate-lines = t  | char-wrap (no word)  |
  |                     |                      |
  +---------------------+---------------------+
  | word-wrap = t       | visual-line-mode     |
  |                     |                      |
  +---------------------+---------------------+

Each window displays its own buffer with the same long-line content
so you can visually compare the rendering side by side."
  (delete-other-windows)
  ;; Build all buffers
  (let ((buf-trunc     (word-wrap-test--buf-truncate))
        (buf-char      (word-wrap-test--buf-char-wrap))
        (buf-word      (word-wrap-test--buf-word-wrap))
        (buf-visual    (word-wrap-test--buf-visual-line)))
    ;; Top-left: truncate
    (switch-to-buffer buf-trunc)
    ;; Split horizontally for top-right
    (let ((w-right (split-window-right)))
      ;; Top-right: char wrap
      (set-window-buffer w-right buf-char)
      ;; Split top-left vertically for bottom-left
      (let ((w-bottom-left (split-window-below)))
        ;; Bottom-left: word wrap
        (set-window-buffer w-bottom-left buf-word)
        ;; Split top-right vertically for bottom-right
        (select-window w-right)
        (let ((w-bottom-right (split-window-below)))
          ;; Bottom-right: visual-line-mode
          (set-window-buffer w-bottom-right buf-visual))))
    ;; Select top-left
    (select-window (frame-first-window))))

;; ---------------------------------------------------------------------------
;; Individual test commands
;; ---------------------------------------------------------------------------

(defun word-wrap-test--show-fill-column ()
  "Show fill-column-indicator test in a single window."
  (interactive)
  (delete-other-windows)
  (switch-to-buffer (word-wrap-test--buf-fill-column))
  (message "Test 7: fill-column-indicator-mode (column 80) with word-wrap"))

(defun word-wrap-test--show-partial-width ()
  "Show truncate-partial-width-windows test with a narrow split."
  (interactive)
  (delete-other-windows)
  (let ((buf (word-wrap-test--buf-partial-width)))
    (switch-to-buffer buf)
    ;; Create a narrow side window to trigger partial-width behavior
    (let ((narrow (split-window-right 40)))
      (set-window-buffer narrow buf))
    (message "Test 8: truncate-partial-width-windows -- left is wide, right is narrow")))

(defun word-wrap-test--show-hscroll ()
  "Show horizontal scrolling test."
  (interactive)
  (delete-other-windows)
  (switch-to-buffer (word-wrap-test--buf-hscroll))
  (message "Test 10: Horizontal scroll -- move cursor right into long lines"))

(defun word-wrap-test--show-4-pane ()
  "Show the 4-pane comparison layout (default)."
  (interactive)
  (word-wrap-test--setup-windows)
  (message "4-pane layout: truncate / char-wrap / word-wrap / visual-line-mode"))

;; ---------------------------------------------------------------------------
;; Test runner
;; ---------------------------------------------------------------------------

(defvar word-wrap-test--tests
  '(("4-pane comparison (truncate/char/word/visual)" . word-wrap-test--show-4-pane)
    ("fill-column-indicator-mode" . word-wrap-test--show-fill-column)
    ("truncate-partial-width-windows" . word-wrap-test--show-partial-width)
    ("horizontal scrolling" . word-wrap-test--show-hscroll))
  "Alist of (name . function) for word-wrap tests.")

(defvar word-wrap-test--step 0
  "Current test step.")

(defun word-wrap-test-next ()
  "Run the next test in the sequence."
  (interactive)
  (when (>= word-wrap-test--step (length word-wrap-test--tests))
    (setq word-wrap-test--step 0))
  (let ((test (nth word-wrap-test--step word-wrap-test--tests)))
    (message "--- Test %d/%d: %s ---"
             (1+ word-wrap-test--step)
             (length word-wrap-test--tests)
             (car test))
    (funcall (cdr test))
    (setq word-wrap-test--step (1+ word-wrap-test--step))))

(defun word-wrap-test-run-all ()
  "Run all tests sequentially with pauses."
  (interactive)
  (setq word-wrap-test--step 0)
  (let ((i 0))
    (dolist (test word-wrap-test--tests)
      (run-at-time (* i 4) nil
                   (lambda (test-pair idx)
                     (message "\n=== Test %d/%d: %s ==="
                              (1+ idx)
                              (length word-wrap-test--tests)
                              (car test-pair))
                     (funcall (cdr test-pair)))
                   test i)
      (setq i (1+ i)))
    (run-at-time (* i 4) nil
                 (lambda ()
                   (message "\n=== All %d word-wrap tests complete ==="
                            (length word-wrap-test--tests))
                   (message "Keys: n=next  a=run-all  1-4=specific test  q=cleanup"))))
  (message "Running all %d tests (4 second intervals)..." (length word-wrap-test--tests)))

(defun word-wrap-test--cleanup ()
  "Kill all test buffers and restore single window."
  (interactive)
  (dolist (name '("*wrap: truncate-lines*"
                  "*wrap: char-wrap*"
                  "*wrap: word-wrap*"
                  "*wrap: visual-line-mode*"
                  "*wrap: fill-column-indicator*"
                  "*wrap: partial-width*"
                  "*wrap: hscroll*"))
    (when (get-buffer name)
      (kill-buffer name)))
  (delete-other-windows)
  (message "Word-wrap test buffers cleaned up."))

;; Keybindings for interactive use
(defvar word-wrap-test-map (make-sparse-keymap)
  "Keymap for word-wrap test.")

(define-key word-wrap-test-map (kbd "n") #'word-wrap-test-next)
(define-key word-wrap-test-map (kbd "a") #'word-wrap-test-run-all)
(define-key word-wrap-test-map (kbd "q") #'word-wrap-test--cleanup)
(define-key word-wrap-test-map (kbd "1") #'word-wrap-test--show-4-pane)
(define-key word-wrap-test-map (kbd "2") #'word-wrap-test--show-fill-column)
(define-key word-wrap-test-map (kbd "3") #'word-wrap-test--show-partial-width)
(define-key word-wrap-test-map (kbd "4") #'word-wrap-test--show-hscroll)

(set-transient-map word-wrap-test-map t)

;; --- Run on load ---
(message "=== Word Wrap & Truncation Test Suite ===")
(message "Keys: n=next  a=run-all  1=4-pane  2=fill-col  3=partial-width  4=hscroll  q=cleanup")
(word-wrap-test--setup-windows)

;;; word-wrap-test.el ends here
