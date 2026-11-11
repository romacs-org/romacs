;;; ligature-test.el --- Test font ligature rendering -*- lexical-binding: t -*-

;; Test whether programming font ligatures render correctly in romacs.
;; Uses JetBrains Mono which has extensive ligature support.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/ligature-test.el
;;
;; Common ligature sequences in programming fonts:
;;   ->  =>  !=  ==  ===  >=  <=  <>  |>  <|  >>  <<
;;   ::  ..  ...  //  /*  */  ;;  --  ++  **  ~~  %%
;;   www  <!--  -->  |||  &&  ||  ?:  ?.  ..=  =>>

;;; Code:

;; Set font to JetBrains Mono (has ligatures)
(set-face-attribute 'default nil
                    :family "JetBrains Mono"
                    :height 140)

(defun ligature-test--insert-section (title)
  "Insert section TITLE."
  (insert "\n")
  (let ((start (point)))
    (insert (format "=== %s ===\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold"))))

(defun ligature-test--insert-pair (label text)
  "Insert LABEL and ligature TEXT sample."
  (insert (format "  %-30s  " label))
  (let ((start (point)))
    (insert text)
    (put-text-property start (point) 'face '(:foreground "cyan" :height 1.5)))
  (insert "\n"))

(defun ligature-test ()
  "Create ligature test buffer."
  (interactive)
  (let ((buf (get-buffer-create "*Ligature Test*")))
    (switch-to-buffer buf)
    (erase-buffer)

    (let ((start (point)))
      (insert "FONT LIGATURE RENDERING TEST\n")
      (put-text-property start (point) 'face '(:weight bold :height 1.8 :foreground "cyan")))
    (insert (format "Font: %s\n" (face-attribute 'default :family)))
    (insert (format "Window system: %s\n" window-system))
    (insert (make-string 70 ?-) "\n")
    (insert "\n")
    (insert "If ligatures work, multi-character sequences below should render as\n")
    (insert "SINGLE connected glyphs, not separate characters.\n")

    ;; Arrow ligatures
    (ligature-test--insert-section "ARROWS")
    (ligature-test--insert-pair "right arrow:"        "->")
    (ligature-test--insert-pair "fat arrow:"          "=>")
    (ligature-test--insert-pair "left arrow:"         "<-")
    (ligature-test--insert-pair "left fat arrow:"     "<=")
    (ligature-test--insert-pair "bidirectional:"      "<->")
    (ligature-test--insert-pair "fat bidirectional:"  "<=>")
    (ligature-test--insert-pair "long right arrow:"   "-->")
    (ligature-test--insert-pair "long left arrow:"    "<--")
    (ligature-test--insert-pair "pipe forward:"       "|>")
    (ligature-test--insert-pair "pipe backward:"      "<|")

    ;; Comparison ligatures
    (ligature-test--insert-section "COMPARISON / EQUALITY")
    (ligature-test--insert-pair "not equal:"          "!=")
    (ligature-test--insert-pair "not identical:"      "!==")
    (ligature-test--insert-pair "equal:"              "==")
    (ligature-test--insert-pair "identical:"          "===")
    (ligature-test--insert-pair "greater-equal:"      ">=")
    (ligature-test--insert-pair "less-equal:"         "<=")
    (ligature-test--insert-pair "diamond:"            "<>")
    (ligature-test--insert-pair "spaceship:"          "<=>")

    ;; Logical ligatures
    (ligature-test--insert-section "LOGICAL OPERATORS")
    (ligature-test--insert-pair "and:"                "&&")
    (ligature-test--insert-pair "or:"                 "||")
    (ligature-test--insert-pair "triple or:"          "|||")
    (ligature-test--insert-pair "ternary:"            "?:")
    (ligature-test--insert-pair "null coalesce:"      "??")
    (ligature-test--insert-pair "optional chain:"     "?.")

    ;; Shift / stream ligatures
    (ligature-test--insert-section "SHIFT / STREAM")
    (ligature-test--insert-pair "left shift:"         "<<")
    (ligature-test--insert-pair "right shift:"        ">>")
    (ligature-test--insert-pair "heredoc:"            "<<<")
    (ligature-test--insert-pair "right shift assign:" ">>=")
    (ligature-test--insert-pair "left shift assign:"  "<<=")

    ;; Scope / type ligatures
    (ligature-test--insert-section "SCOPE / TYPE")
    (ligature-test--insert-pair "scope:"              "::")
    (ligature-test--insert-pair "assign type:"        ":=")
    (ligature-test--insert-pair "range:"              "..")
    (ligature-test--insert-pair "spread:"             "...")
    (ligature-test--insert-pair "range inclusive:"     "..=")

    ;; Comment ligatures
    (ligature-test--insert-section "COMMENTS / STRINGS")
    (ligature-test--insert-pair "line comment:"       "//")
    (ligature-test--insert-pair "block comment open:" "/*")
    (ligature-test--insert-pair "block comment close:" "*/")
    (ligature-test--insert-pair "HTML comment open:"  "<!--")
    (ligature-test--insert-pair "HTML comment close:" "-->")
    (ligature-test--insert-pair "hash-bang:"          "#!")
    (ligature-test--insert-pair "string escape:"      "\\n")

    ;; Arithmetic ligatures
    (ligature-test--insert-section "ARITHMETIC / MISC")
    (ligature-test--insert-pair "increment:"          "++")
    (ligature-test--insert-pair "decrement:"          "--")
    (ligature-test--insert-pair "exponent:"           "**")
    (ligature-test--insert-pair "tilde-tilde:"        "~~")
    (ligature-test--insert-pair "plus-equal:"         "+=")
    (ligature-test--insert-pair "minus-equal:"        "-=")
    (ligature-test--insert-pair "multiply-equal:"     "*=")
    (ligature-test--insert-pair "divide-equal:"       "/=")

    ;; Haskell / functional
    (ligature-test--insert-section "HASKELL / FUNCTIONAL")
    (ligature-test--insert-pair "bind:"               ">>="
    )
    (ligature-test--insert-pair "apply:"              "<*>")
    (ligature-test--insert-pair "fmap:"               "<$>")
    (ligature-test--insert-pair "alternative:"        "<|>")
    (ligature-test--insert-pair "monad:"              ">=>")
    (ligature-test--insert-pair "kleisli:"            "<=<")
    (ligature-test--insert-pair "type constraint:"    "=>")

    ;; Real code samples
    (ligature-test--insert-section "REAL CODE SAMPLES")
    (insert "\n")
    (let ((start (point)))
      (insert "  // Rust\n")
      (insert "  fn main() -> Result<(), Box<dyn Error>> {\n")
      (insert "      let x = vec![1, 2, 3];\n")
      (insert "      let y = x.iter().map(|&n| n != 0 && n >= 2).collect::<Vec<_>>();\n")
      (insert "      x == y || x <= y;\n")
      (insert "      println!(\"result: {:?}\", y);\n")
      (insert "  }\n")
      (insert "\n")
      (insert "  // Haskell\n")
      (insert "  main :: IO ()\n")
      (insert "  main = getLine >>= putStrLn . map toUpper\n")
      (insert "  compose = (.) . (.)\n")
      (insert "\n")
      (insert "  // JavaScript\n")
      (insert "  const fn = (x) => x !== null ? x?.value ?? 0 : -1;\n")
      (insert "  const cmp = a === b || a >= c && a <= d;\n")
      (put-text-property start (point) 'face '(:family "JetBrains Mono")))

    ;; ---- Section: Cursor movement through ligature sequences ----
    (ligature-test--insert-section "CURSOR MOVEMENT THROUGH LIGATURES")
    (insert "\n")
    (insert "  Move point through ligature sequences below. The cursor should\n")
    (insert "  advance character-by-character even when glyphs are merged.\n")
    (insert "  Markers: | = cursor position (visual only)\n\n")
    (let ((seqs '(("->" . "right arrow")
                  ("=>" . "fat arrow")
                  ("!==" . "not identical")
                  ("===" . "strict equal")
                  ("<=>" . "spaceship")
                  (">>=" . "bind")
                  ("<!--" . "HTML comment open")
                  ("-->" . "HTML comment close"))))
      (dolist (pair seqs)
        (let* ((seq (car pair))
               (label (cdr pair))
               (len (length seq)))
          (insert (format "  %-20s  " label))
          ;; Show cursor at each position: |-> -|> ->|
          (dotimes (i (1+ len))
            (let ((before (substring seq 0 i))
                  (after (substring seq i)))
              (insert before)
              (let ((bar-start (point)))
                (insert "|")
                (put-text-property bar-start (point) 'face '(:foreground "red" :weight bold)))
              (let ((start (point)))
                (insert after)
                (put-text-property start (point) 'face '(:foreground "cyan")))
              (when (< i len)
                (insert "  "))))
          (insert "\n"))))
    (insert "\n")
    (insert "  Interactive: place point in the sequence below and use C-f / C-b.\n")
    (insert "  ")
    (let ((start (point)))
      (insert "a]->=>!==<=>--><!--[b")
      (put-text-property start (point) 'face '(:foreground "yellow" :height 1.5)))
    (insert "\n")

    ;; ---- Section: Face changes mid-ligature ----
    (ligature-test--insert-section "FACE CHANGES MID-LIGATURE")
    (insert "\n")
    (insert "  When a face change occurs inside a ligature sequence, the ligature\n")
    (insert "  should break into separate glyphs (no single merged glyph).\n\n")
    (let ((test-cases
           '(("->" "right arrow" (1 . (:foreground "red")))
             ("=>" "fat arrow" (1 . (:foreground "green")))
             ("!=" "not equal" (1 . (:foreground "magenta")))
             ("==" "double equal" (1 . (:foreground "orange")))
             ("===" "triple equal" (1 . (:foreground "red")))
             ("===" "triple equal mid" (2 . (:foreground "red")))
             ("<=>" "spaceship" (1 . (:foreground "yellow")))
             ("<!--" "HTML open" (2 . (:foreground "lime green")))
             (">>=" "bind" (1 . (:foreground "deep sky blue"))))))
      (dolist (tc test-cases)
        (let* ((seq (nth 0 tc))
               (label (nth 1 tc))
               (split-spec (nth 2 tc))
               (split-at (car split-spec))
               (split-face (cdr split-spec))
               (part1 (substring seq 0 split-at))
               (part2 (substring seq split-at)))
          ;; Intact ligature for comparison
          (insert (format "  %-22s intact: " label))
          (let ((start (point)))
            (insert seq)
            (put-text-property start (point) 'face '(:foreground "cyan" :height 1.3)))
          ;; Split ligature
          (insert "    split at ")
          (insert (format "%d: " split-at))
          (let ((start (point)))
            (insert part1)
            (put-text-property start (point) 'face split-face))
          (let ((start (point)))
            (insert part2)
            (put-text-property start (point) 'face '(:foreground "cyan")))
          (insert "\n"))))
    (insert "\n")
    (insert "  The 'intact' column should show a merged ligature glyph.\n")
    (insert "  The 'split' column should show separate characters (ligature broken).\n")

    ;; ---- Section: Overlays on ligature sequences ----
    (ligature-test--insert-section "OVERLAYS ON LIGATURE SEQUENCES")
    (insert "\n")
    (insert "  Overlays with face properties applied over ligature sequences.\n")
    (insert "  Check that overlays interact correctly (highlight, color change).\n\n")
    (let ((overlay-tests
           '(("->  =>  !==  ===  <=>  -->  <!--"
              "full highlight"
              (:background "dark green" :foreground "white"))
             ("->  =>  !==  ===  <=>  -->  <!--"
              "underline"
              (:underline t :foreground "cyan"))
             ("->  =>  !==  ===  <=>  -->  <!--"
              "bold + color"
              (:weight bold :foreground "orange red"))
             ("->  =>  !==  ===  <=>  -->  <!--"
              "inverse video"
              (:inverse-video t)))))
      (dolist (ot overlay-tests)
        (let* ((text (nth 0 ot))
               (label (nth 1 ot))
               (face (nth 2 ot)))
          (insert (format "  %-18s  " label))
          (let ((start (point)))
            (insert text)
            (let ((ov (make-overlay start (point))))
              (overlay-put ov 'face face)))
          (insert "\n"))))
    (insert "\n")
    ;; Partial overlay: overlay covers only part of a ligature
    (insert "  Partial overlays (overlay covers only part of the sequence):\n")
    (let ((partial-tests
           '(("->" 0 1 "1st char of ->"  (:background "dark red"))
             ("->" 1 2 "2nd char of ->"  (:background "dark blue"))
             ("===" 0 2 "first 2 of ===" (:background "dark magenta"))
             ("===" 1 3 "last 2 of ==="  (:background "dark cyan"))
             ("<!--" 0 2 "first 2 of <!--" (:background "dark green"))
             ("<!--" 2 4 "last 2 of <!--"  (:background "midnight blue")))))
      (dolist (pt partial-tests)
        (let* ((seq (nth 0 pt))
               (ov-start-off (nth 1 pt))
               (ov-end-off (nth 2 pt))
               (label (nth 3 pt))
               (face (nth 4 pt)))
          (insert (format "  %-22s  " label))
          (let ((start (point)))
            (insert seq)
            (let ((ov (make-overlay (+ start ov-start-off) (+ start ov-end-off))))
              (overlay-put ov 'face face)))
          (insert "\n"))))
    (insert "\n")
    ;; before-string / after-string overlays
    (insert "  Overlay before/after-string on ligatures:\n")
    (let ((start (point)))
      (insert "  ->")
      (let ((ov (make-overlay start (+ start 4))))
        (overlay-put ov 'before-string
                     (propertize "[B]" 'face '(:foreground "green")))
        (overlay-put ov 'after-string
                     (propertize "[A]" 'face '(:foreground "red")))))
    (insert "  ")
    (let ((start (point)))
      (insert "=>")
      (let ((ov (make-overlay start (+ start 2))))
        (overlay-put ov 'before-string
                     (propertize "[B]" 'face '(:foreground "green")))
        (overlay-put ov 'after-string
                     (propertize "[A]" 'face '(:foreground "red")))))
    (insert "  ")
    (let ((start (point)))
      (insert "!==")
      (let ((ov (make-overlay start (+ start 3))))
        (overlay-put ov 'before-string
                     (propertize "[B]" 'face '(:foreground "green")))
        (overlay-put ov 'after-string
                     (propertize "[A]" 'face '(:foreground "red")))))
    (insert "\n")

    ;; ---- Section: Ligatures in different major modes ----
    (ligature-test--insert-section "LIGATURES IN DIFFERENT MAJOR MODES")
    (insert "\n")
    (insert "  Each sub-buffer below uses a different major mode.\n")
    (insert "  Ligature rendering should work regardless of major mode.\n\n")
    (let ((mode-tests
           '((fundamental-mode "fundamental-mode"
              "int x = (a != b) && (c >= d) || (e <= f);\nfn main() -> Result<()> { Ok(()) }\n")
             (prog-mode "prog-mode"
              "let cmp = a === b || a !== c;\nconst fn = (x) => x ?? 0;\n")
             (emacs-lisp-mode "emacs-lisp-mode"
              ";; This is a comment with ligatures: -> => != == >=\n(defun foo (x) (if (/= x 0) (<= x 10) nil))\n")
             (text-mode "text-mode"
              "Arrows: -> => <-> <=> --> <--\nComparisons: == != === !== >= <=\n"))))
      (dolist (mt mode-tests)
        (let* ((mode (nth 0 mt))
               (mode-name-str (nth 1 mt))
               (code (nth 2 mt))
               (buf-name (format " *ligature-%s*" mode-name-str)))
          ;; Create a temporary buffer in the target mode
          (let ((tbuf (get-buffer-create buf-name)))
            (with-current-buffer tbuf
              (erase-buffer)
              (insert code)
              (funcall mode)
              (font-lock-ensure)))
          ;; In the test buffer, show the mode name and the code
          (insert (format "  [%s]\n" mode-name-str))
          (let ((start (point)))
            (insert code)
            (put-text-property start (point) 'face '(:foreground "light steel blue")))
          (insert "\n"))))
    (insert "  Note: to see full font-lock interaction, open the temp buffers:\n")
    (insert "    M-x switch-to-buffer RET  *ligature-prog-mode* RET\n")

    ;; ---- Section: Toggle ligatures on/off ----
    (ligature-test--insert-section "TOGGLE LIGATURES (romacs-set-ligatures-enabled)")
    (insert "\n")
    (if (fboundp 'romacs-set-ligatures-enabled)
        (progn
          (insert "  The function `romacs-set-ligatures-enabled' is available.\n")
          (insert "  Use the keybindings below to toggle ligatures and compare:\n\n")
          (insert "    C-c l e   -- Enable ligatures\n")
          (insert "    C-c l d   -- Disable ligatures\n")
          (insert "    C-c l t   -- Toggle ligatures\n\n")
          ;; Define toggle commands in the test buffer
          (local-set-key (kbd "C-c l e")
                         (lambda () (interactive)
                           (romacs-set-ligatures-enabled t)
                           (message "Ligatures ENABLED")))
          (local-set-key (kbd "C-c l d")
                         (lambda () (interactive)
                           (romacs-set-ligatures-enabled nil)
                           (message "Ligatures DISABLED")))
          (local-set-key (kbd "C-c l t")
                         (lambda () (interactive)
                           (let ((new-state (not (bound-and-true-p romacs-ligatures-enabled))))
                             (setq romacs-ligatures-enabled new-state)
                             (romacs-set-ligatures-enabled new-state)
                             (message "Ligatures %s" (if new-state "ENABLED" "DISABLED")))))
          (insert "  Reference sequences (toggle and watch these change):\n")
          (insert "  ")
          (let ((start (point)))
            (insert "->  =>  !=  ==  ===  >=  <=  <>  |>  >>  <<  ::  ..  ...  -->  <--")
            (put-text-property start (point) 'face '(:foreground "cyan" :height 1.5)))
          (insert "\n"))
      (insert "  `romacs-set-ligatures-enabled' is NOT available in this build.\n")
      (insert "  Ligature toggling is not supported.\n"))

    (insert "\n")
    (ligature-test--insert-section "VERDICT")
    (insert "\n")
    (insert "  If you see connected/merged glyphs for sequences like -> => != ==\n")
    (insert "  then ligatures ARE working.\n")
    (insert "\n")
    (insert "  If -> looks like two separate chars '-' and '>' then ligatures\n")
    (insert "  are NOT working (character-by-character rendering).\n")

    (goto-char (point-min))
    (setq buffer-read-only t)
    (message "Ligature test ready. Check if multi-char sequences render as single glyphs.")))

(ligature-test)

;;; ligature-test.el ends here
