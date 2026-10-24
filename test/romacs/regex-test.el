;;; regex-test.el --- Test regex search and highlighting -*- lexical-binding: t -*-

;; Test regex search and match highlighting, useful for verifying the Rust
;; regex engine integration in romacs.  Each section inserts sample text,
;; searches with a regex pattern, and highlights matches with colored overlays.
;; Usage: ./target/release/romacs -Q -l test/romacs/regex-test.el

;;; Code:

(defvar regex-test--overlays nil
  "List of overlays created during the test.")

(defvar regex-test--step 0
  "Current test step for the interactive sequence.")

(defvar regex-test--buffer-name "*Regex Test*"
  "Name of the test buffer.")

;; ---------------------------------------------------------------------------
;; Helpers
;; ---------------------------------------------------------------------------

(defun regex-test--cleanup-overlays ()
  "Remove all test overlays."
  (dolist (ov regex-test--overlays)
    (when (overlay-buffer ov)
      (delete-overlay ov)))
  (setq regex-test--overlays nil))

(defun regex-test--highlight-matches (pattern face &optional limit)
  "Highlight all matches of PATTERN in the current buffer with FACE.
Search starts from point-min.  Optional LIMIT restricts how many
matches to highlight.  Return the match count."
  (let ((count 0))
    (save-excursion
      (goto-char (point-min))
      (while (and (re-search-forward pattern nil t)
                  (or (null limit) (< count limit)))
        (let ((ov (make-overlay (match-beginning 0) (match-end 0))))
          (overlay-put ov 'face face)
          (push ov regex-test--overlays))
        (setq count (1+ count))))
    count))

(defun regex-test--insert-heading (title)
  "Insert a section heading TITLE with styling."
  (insert "\n")
  (let ((start (point)))
    (insert (format "=== %s ===\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold" :height 1.2))))

(defun regex-test--insert-subheading (text)
  "Insert a sub-heading TEXT with subtle styling."
  (let ((start (point)))
    (insert (format "--- %s ---\n" text))
    (put-text-property start (point) 'face '(:foreground "light steel blue" :weight bold))))

(defun regex-test--insert-pattern-info (pattern expected-count actual-count)
  "Insert a line showing PATTERN, EXPECTED-COUNT and ACTUAL-COUNT."
  (insert "  Pattern: ")
  (let ((start (point)))
    (insert (format "%s" pattern))
    (put-text-property start (point) 'face '(:foreground "cyan" :weight bold)))
  (insert (format "    expected: %d  actual: " expected-count))
  (let ((start (point))
        (ok (= expected-count actual-count)))
    (insert (format "%d %s" actual-count (if ok "OK" "MISMATCH")))
    (put-text-property start (point) 'face
                       (if ok '(:foreground "lime green" :weight bold)
                         '(:foreground "red" :weight bold))))
  (insert "\n"))

(defun regex-test--insert-sample (text)
  "Insert sample TEXT with a dim face to distinguish from headings."
  (let ((start (point)))
    (insert text)
    (put-text-property start (point) 'face '(:foreground "gray70"))))

;; ---------------------------------------------------------------------------
;; Section 1: Basic literal matching
;; ---------------------------------------------------------------------------

(defun regex-test--section-literal ()
  "Test basic literal string matching."
  (regex-test--insert-heading "1. BASIC LITERAL MATCHING")
  (insert "  Match fixed strings in sample text.\n\n")
  (let ((sample "The fox and the dog.  The Fox is fast.  the fox sleeps.\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    ;; case-sensitive
    (let ((n (regex-test--highlight-matches
              "fox" '(:background "#442200" :foreground "orange"))))
      (regex-test--insert-pattern-info "fox" 2 n))
    ;; case-insensitive via Emacs binding
    (let ((case-fold-search t))
      (let ((n (regex-test--highlight-matches
                "fox" '(:background "#003344" :foreground "sky blue"))))
        (regex-test--insert-pattern-info "fox (case-fold)" 3 n)))))

;; ---------------------------------------------------------------------------
;; Section 2: Character classes [a-z] [0-9]
;; ---------------------------------------------------------------------------

(defun regex-test--section-char-classes ()
  "Test character class matching."
  (regex-test--insert-heading "2. CHARACTER CLASSES [a-z], [0-9]")
  (insert "  Match character ranges and custom classes.\n\n")
  (let ((sample "abc 123 XYZ !@# def 456 GHI 789\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "[a-z]+" '(:background "#002200" :foreground "lime green"))))
      (regex-test--insert-pattern-info "[a-z]+" 2 n))
    (let ((n (regex-test--highlight-matches
              "[0-9]+" '(:background "#220022" :foreground "orchid"))))
      (regex-test--insert-pattern-info "[0-9]+" 3 n))
    (let ((n (regex-test--highlight-matches
              "[A-Z]+" '(:background "#002244" :foreground "deep sky blue"))))
      (regex-test--insert-pattern-info "[A-Z]+" 2 n))
    (let ((n (regex-test--highlight-matches
              "[!@#]+" '(:background "#442200" :foreground "gold"))))
      (regex-test--insert-pattern-info "[!@#]+" 1 n))))

;; ---------------------------------------------------------------------------
;; Section 3: Wildcards . and .*
;; ---------------------------------------------------------------------------

(defun regex-test--section-wildcards ()
  "Test dot wildcard and greedy matching."
  (regex-test--insert-heading "3. WILDCARDS . AND .*")
  (insert "  Dot matches any character (except newline by default).\n\n")
  (let ((sample "cat cot cut c3t c_t c t\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "c.t" '(:background "#330033" :foreground "magenta"))))
      (regex-test--insert-pattern-info "c.t" 5 n)))
  (let ((sample "start---middle---end\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    ;; Greedy .* matches the longest span
    (let ((n (regex-test--highlight-matches
              "start.*end" '(:background "#003300" :foreground "green"))))
      (regex-test--insert-pattern-info "start.*end" 1 n))
    ;; Non-greedy .*? (Emacs supports via \?)
    (let ((n (regex-test--highlight-matches
              "start.*?-" '(:background "#333300" :foreground "yellow"))))
      (regex-test--insert-pattern-info "start.*?- (non-greedy first dash)" 1 n))))

;; ---------------------------------------------------------------------------
;; Section 4: Anchors ^ and $
;; ---------------------------------------------------------------------------

(defun regex-test--section-anchors ()
  "Test line anchors."
  (regex-test--insert-heading "4. ANCHORS ^ AND $")
  (insert "  Match at beginning/end of lines.\n\n")
  (let ((sample "alpha bravo\ncharlie delta\nalpha echo\nfoxtrot alpha\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "^alpha" '(:background "#440000" :foreground "salmon"))))
      (regex-test--insert-pattern-info "^alpha" 2 n))
    (let ((n (regex-test--highlight-matches
              "alpha$" '(:background "#000044" :foreground "light blue"))))
      (regex-test--insert-pattern-info "alpha$" 1 n))
    (let ((n (regex-test--highlight-matches
              "^.*$" '(:background "#222222" :foreground "white"))))
      (regex-test--insert-pattern-info "^.*$ (full lines)" 4 n))))

;; ---------------------------------------------------------------------------
;; Section 5: Bounded repetitions \{n,m\}
;; ---------------------------------------------------------------------------

(defun regex-test--section-bounded-repetition ()
  "Test bounded repetition operators -- the newly implemented feature."
  (regex-test--insert-heading "5. BOUNDED REPETITIONS \\{n,m\\} (NEW FEATURE)")
  (insert "  Test the newly implemented bounded repetition quantifiers.\n\n")

  (regex-test--insert-subheading "Exact count \\{n\\}")
  (let ((sample "a aa aaa aaaa aaaaa\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "a\\{3\\}" '(:background "#004400" :foreground "lime green"))))
      (regex-test--insert-pattern-info "a\\{3\\}" 3 n)))

  (regex-test--insert-subheading "Range \\{n,m\\}")
  (let ((sample "ab aab aaab aaaab aaaaab\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "a\\{2,4\\}b" '(:background "#440044" :foreground "orchid"))))
      (regex-test--insert-pattern-info "a\\{2,4\\}b" 3 n)))

  (regex-test--insert-subheading "Minimum only \\{n,\\}")
  (let ((sample "x xx xxx xxxx xxxxx\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "x\\{3,\\}" '(:background "#443300" :foreground "gold"))))
      (regex-test--insert-pattern-info "x\\{3,\\}" 3 n)))

  (regex-test--insert-subheading "Zero-or-more \\{0,\\} (same as *)")
  (let ((sample "b ab aab aaab\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "a\\{0,\\}b" '(:background "#003344" :foreground "cyan"))))
      (regex-test--insert-pattern-info "a\\{0,\\}b" 4 n)))

  (regex-test--insert-subheading "Digits with bounded repetition")
  (let ((sample "1 12 123 1234 12345 123456\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "[0-9]\\{3,5\\}" '(:background "#330000" :foreground "tomato"))))
      (regex-test--insert-pattern-info "[0-9]\\{3,5\\}" 4 n))))

;; ---------------------------------------------------------------------------
;; Section 6: Word boundaries \b
;; ---------------------------------------------------------------------------

(defun regex-test--section-word-boundaries ()
  "Test word boundary matching."
  (regex-test--insert-heading "6. WORD BOUNDARIES \\b")
  (insert "  \\b matches at word/non-word transitions.\n")
  (insert "  (Emacs uses \\< for word-start, \\> for word-end.)\n\n")
  (let ((sample "the cat concatenate theater themed cathode\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    ;; \bthe\b matches only whole word "the"
    (let ((n (regex-test--highlight-matches
              "\\bthe\\b" '(:background "#004400" :foreground "lime green"))))
      (regex-test--insert-pattern-info "\\bthe\\b" 1 n))
    ;; Emacs-style: \< = word start, \> = word end
    (let ((n (regex-test--highlight-matches
              "\\<cat\\>" '(:background "#440044" :foreground "orchid"))))
      (regex-test--insert-pattern-info "\\<cat\\> (whole word)" 1 n))
    ;; Word start only -- matches cat, concatenate, cathode
    (let ((n (regex-test--highlight-matches
              "\\<cat" '(:background "#002244" :foreground "deep sky blue"))))
      (regex-test--insert-pattern-info "\\<cat (word-start)" 3 n))
    ;; Word end only -- matches the, concatenate
    (let ((n (regex-test--highlight-matches
              "the\\>" '(:background "#442200" :foreground "orange"))))
      (regex-test--insert-pattern-info "the\\> (word-end)" 1 n))))

;; ---------------------------------------------------------------------------
;; Section 7: Groups \(...\) and backreferences \1
;; ---------------------------------------------------------------------------

(defun regex-test--section-groups-backrefs ()
  "Test grouping and backreferences."
  (regex-test--insert-heading "7. GROUPS \\(...\\) AND BACKREFERENCES \\1")
  (insert "  Capture groups and match the same text again.\n\n")
  (let ((sample "abab cdcd efgh abcd xyxy aaaa xyzxyz\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    ;; Match doubled two-char sequences: abab, cdcd, xyxy
    (let ((n (regex-test--highlight-matches
              "\\([a-z]\\{2\\}\\)\\1"
              '(:background "#004444" :foreground "cyan"))))
      (regex-test--insert-pattern-info "\\([a-z]\\{2\\}\\)\\1 (doubled pair)" 4 n)))
  (let ((sample "<b>bold</b>  <i>italic</i>  <b>mismatch</i>\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    ;; Match balanced HTML tags: <b>...</b> and <i>...</i>
    (let ((n (regex-test--highlight-matches
              "<\\([a-z]+\\)>[^<]*</\\1>"
              '(:background "#002244" :foreground "sky blue"))))
      (regex-test--insert-pattern-info "<\\([a-z]+\\)>...<\\/\\1> (balanced tags)" 2 n))))

;; ---------------------------------------------------------------------------
;; Section 8: Alternation \|
;; ---------------------------------------------------------------------------

(defun regex-test--section-alternation ()
  "Test alternation (or) operator."
  (regex-test--insert-heading "8. ALTERNATION \\|")
  (insert "  Match one of several alternatives.\n\n")
  (let ((sample "apple banana cherry date elderberry fig grape\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "apple\\|cherry\\|grape"
              '(:background "#003300" :foreground "lime green"))))
      (regex-test--insert-pattern-info "apple\\|cherry\\|grape" 3 n)))
  (let ((sample "error: file not found\nwarning: deprecated API\ninfo: build complete\nerror: timeout\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    (let ((n (regex-test--highlight-matches
              "^\\(error\\|warning\\):"
              '(:background "#440000" :foreground "salmon"))))
      (regex-test--insert-pattern-info "^\\(error\\|warning\\):" 3 n))))

;; ---------------------------------------------------------------------------
;; Section 9: Emacs-specific: \w, \s-, \sw
;; ---------------------------------------------------------------------------

(defun regex-test--section-emacs-specific ()
  "Test Emacs-specific regex syntax classes."
  (regex-test--insert-heading "9. EMACS-SPECIFIC: \\w \\s- \\sw")
  (insert "  Emacs syntax-table-aware character classes.\n")
  (insert "  \\w = word constituent, \\W = non-word\n")
  (insert "  \\sw = word syntax, \\s- = whitespace syntax\n")
  (insert "  \\s. = punctuation syntax\n\n")

  (let ((sample "hello world  42 foo_bar!  (list 'a \"str\")\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    ;; \w+ matches word-constituent runs
    (let ((n (regex-test--highlight-matches
              "\\w+" '(:background "#002200" :foreground "lime green"))))
      (regex-test--insert-pattern-info "\\w+" 6 n))
    ;; \s- matches whitespace characters
    (let ((n (regex-test--highlight-matches
              "\\s-+" '(:background "#333333" :foreground "gray90"))))
      (regex-test--insert-pattern-info "\\s-+" 6 n)))

  (let ((sample "defun my-func (x y) + - * / = < >\n"))
    (regex-test--insert-sample sample)
    (insert "\n")
    ;; \sw+ matches word-syntax runs (same as \w+ in most cases)
    (let ((n (regex-test--highlight-matches
              "\\sw+" '(:background "#220022" :foreground "plum"))))
      (regex-test--insert-pattern-info "\\sw+" 4 n))
    ;; \s. matches punctuation-syntax characters
    (let ((n (regex-test--highlight-matches
              "\\s." '(:background "#442200" :foreground "orange"))))
      (regex-test--insert-pattern-info "\\s." 9 n))))

;; ---------------------------------------------------------------------------
;; Section 10: Interactive isearch-forward-regexp demo
;; ---------------------------------------------------------------------------

(defun regex-test--section-isearch-demo ()
  "Insert text and instructions for interactive isearch-forward-regexp."
  (regex-test--insert-heading "10. INTERACTIVE isearch-forward-regexp DEMO")
  (insert "  Use C-M-s (isearch-forward-regexp) to test live regex matching.\n")
  (insert "  Try the suggested patterns below on the sample text.\n\n")

  (regex-test--insert-subheading "Sample Text")
  (let ((sample "\
192.168.1.1     server-alpha    active   2026-01-15  CPU: 45%
10.0.0.42       server-beta     standby  2026-02-01  CPU: 12%
172.16.0.100    server-gamma    active   2026-01-28  CPU: 89%
192.168.1.55    server-delta    error    2026-02-10  CPU: 0%
10.0.0.7        server-epsilon  active   2026-02-11  CPU: 67%
255.255.255.0   netmask         ---      ---         ---
fe80::1         ipv6-link-local active   2026-02-11  CPU: 33%
user@host.com   admin-contact   ---      ---         ---
ERROR: disk full on /dev/sda1 at 2026-02-11T03:14:00
WARNING: memory usage 95% on server-gamma
INFO: backup completed successfully at 2026-02-10T22:00:00
"))
    (regex-test--insert-sample sample))

  (insert "\n")
  (regex-test--insert-subheading "Suggested Patterns for C-M-s")
  (insert "\n")
  (let ((suggestions
         '(("[0-9]\\{1,3\\}\\.[0-9]\\{1,3\\}\\.[0-9]\\{1,3\\}\\.[0-9]\\{1,3\\}"
            "IPv4 addresses")
           ("server-[a-z]+"
            "Server hostnames")
           ("\\(active\\|standby\\|error\\)"
            "Status values")
           ("CPU: [0-9]\\{2,3\\}%"
            "CPU usage >= 10%")
           ("^\\(ERROR\\|WARNING\\):"
            "Log severity prefixes")
           ("2026-[0-9]\\{2\\}-[0-9]\\{2\\}"
            "Date stamps (YYYY-MM-DD)")
           ("\\b[a-z0-9.]+@[a-z0-9.]+\\.[a-z]\\{2,\\}\\b"
            "Email addresses")
           ("\\bserver-\\(alpha\\|gamma\\)\\b"
            "Specific servers via alternation")
           ("/$"
            "Lines ending with /")
           ("^[A-Z]\\{2,\\}:"
            "Uppercase log prefixes"))))
    (dolist (s suggestions)
      (insert "  ")
      (let ((start (point)))
        (insert (format "%-60s" (car s)))
        (put-text-property start (point) 'face '(:foreground "cyan")))
      (let ((start (point)))
        (insert (format " -- %s" (cadr s)))
        (put-text-property start (point) 'face '(:foreground "gray60")))
      (insert "\n")))
  (insert "\n")
  (insert "  Press C-M-s, then type a pattern above.  Matches highlight live.\n")
  (insert "  Press C-s to move to next match, C-r for previous, RET to exit.\n"))

;; ---------------------------------------------------------------------------
;; Test list and runner
;; ---------------------------------------------------------------------------

(defvar regex-test--tests
  '(("Literal matching"      . regex-test--section-literal)
    ("Character classes"      . regex-test--section-char-classes)
    ("Wildcards"              . regex-test--section-wildcards)
    ("Anchors"                . regex-test--section-anchors)
    ("Bounded repetitions"    . regex-test--section-bounded-repetition)
    ("Word boundaries"        . regex-test--section-word-boundaries)
    ("Groups & backrefs"      . regex-test--section-groups-backrefs)
    ("Alternation"            . regex-test--section-alternation)
    ("Emacs-specific syntax"  . regex-test--section-emacs-specific)
    ("isearch-forward-regexp" . regex-test--section-isearch-demo))
  "Alist of (name . function) for regex test sections.")

(defun regex-test-run-all ()
  "Run all regex test sections and display results in a single buffer."
  (interactive)
  (let ((buf (get-buffer-create regex-test--buffer-name)))
    (switch-to-buffer buf)
    (let ((inhibit-read-only t))
      (erase-buffer)
      (regex-test--cleanup-overlays)

      ;; Title
      (let ((start (point)))
        (insert "REGEX SEARCH AND HIGHLIGHTING TEST\n")
        (put-text-property start (point) 'face '(:weight bold :height 1.8 :foreground "cyan")))
      (insert (format "Window system: %s\n" window-system))
      (insert (make-string 72 ?-) "\n")
      (insert "\n")
      (insert "Each section creates sample text, runs regex searches, and highlights\n")
      (insert "matches with colored overlays.  Pattern info shows expected vs actual\n")
      (insert "match counts.  All 'OK' means the regex engine is working correctly.\n")

      ;; Run each section
      (dolist (test regex-test--tests)
        (funcall (cdr test)))

      ;; Summary
      (insert "\n")
      (let ((start (point)))
        (insert (make-string 72 ?=) "\n")
        (put-text-property start (point) 'face '(:foreground "gold")))
      (let ((start (point)))
        (insert (format "Total overlays created: %d\n" (length regex-test--overlays)))
        (put-text-property start (point) 'face '(:weight bold :foreground "lime green")))
      (insert "Scroll through sections above to verify highlighted matches.\n")
      (insert "Use C-M-s on the sample text in section 10 for interactive testing.\n")

      (goto-char (point-min))
      (setq buffer-read-only t)))
  (message "Regex test complete.  %d overlays highlighting matches."
           (length regex-test--overlays)))

(defun regex-test-run-section (n)
  "Run a single test section N (1-indexed) in a fresh buffer."
  (interactive "nSection number (1-10): ")
  (when (or (< n 1) (> n (length regex-test--tests)))
    (error "Section %d out of range (1-%d)" n (length regex-test--tests)))
  (let ((buf (get-buffer-create regex-test--buffer-name))
        (test (nth (1- n) regex-test--tests)))
    (switch-to-buffer buf)
    (let ((inhibit-read-only t))
      (erase-buffer)
      (regex-test--cleanup-overlays)
      (let ((start (point)))
        (insert (format "REGEX TEST: Section %d — %s\n" n (car test)))
        (put-text-property start (point) 'face '(:weight bold :height 1.5 :foreground "cyan")))
      (insert (make-string 60 ?-) "\n")
      (funcall (cdr test))
      (goto-char (point-min))
      (setq buffer-read-only t)))
  (message "Section %d: %d overlays." n (length regex-test--overlays)))

(defun regex-test-next ()
  "Run the next test section in sequence."
  (interactive)
  (when (>= regex-test--step (length regex-test--tests))
    (setq regex-test--step 0))
  (setq regex-test--step (1+ regex-test--step))
  (regex-test-run-section regex-test--step))

(defun regex-test-cleanup ()
  "Remove all overlays and kill the test buffer."
  (interactive)
  (regex-test--cleanup-overlays)
  (when (get-buffer regex-test--buffer-name)
    (kill-buffer regex-test--buffer-name))
  (message "Regex test cleaned up."))

;; Keybindings for interactive use
(defvar regex-test-map (make-sparse-keymap)
  "Keymap for regex test.")

(define-key regex-test-map (kbd "a") #'regex-test-run-all)
(define-key regex-test-map (kbd "n") #'regex-test-next)
(define-key regex-test-map (kbd "q") #'regex-test-cleanup)
(dotimes (i 10)
  (define-key regex-test-map (kbd (format "%d" (if (= i 9) 0 (1+ i))))
    (let ((section (if (= i 9) 10 (1+ i))))
      (lambda () (interactive) (regex-test-run-section section)))))

(set-transient-map regex-test-map t)

;; Setup on load
(message "=== Regex Search & Highlighting Test ===")
(message "Keys: a=run-all  n=next  1-9,0=section  q=cleanup")
(regex-test-run-all)

;;; regex-test.el ends here
