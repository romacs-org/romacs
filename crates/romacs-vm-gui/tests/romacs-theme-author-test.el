;;; romacs-theme-author-test.el --- `def-romacs-theme' and the role table -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-theme.el's author-facing macro and
;; rlisp/romacs-theme-roles.el, in a `--loadup' session
;; (gnu_completion.rs `romacs_theme_author_tests_pass').

(require 'romacs-theme)
(require 'romacs-theme-roles)

(defconst romacs-theme-author-test--root
  (expand-file-name "../../../" (file-name-directory load-file-name))
  "The repository.")

(def-romacs-theme romacs-author-dark "A short dark palette." :mode dark :family romacs-author :style flat
  ((bg "#101418") (fg "#d8dee9") (red "#e06c75") (green "#98c379") (yellow "#e5c07b")
   (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2")
   (bg-alt (romacs-theme-lighten bg 0.05)) (keyword magenta) (comment "#7f8794"))
  ((region :background (romacs-theme-blend bg blue 0.3))
   (romacs-comment :foreground red)
   (show-paren-match &dark (:underline t) &light (:overline t))
   (font-lock-string-face :box (:line-width 1 :color green)))
  ((romacs-author-variable (romacs-theme-color 'red))))

(def-romacs-theme romacs-author-light "The same family, light." :family romacs-author
  ((bg "#fafafa") (fg "#2a2f38") (red "#c0343c") (green "#3e7d2f") (yellow "#8a6a00")
   (blue "#2a62c9") (magenta "#8b3fb5") (cyan "#1f7f8c"))
  ((show-paren-match &dark (:underline t) &light (:overline t))))

(defun romacs-theme-author-test--row (theme face)
  "THEME's rows' attributes for FACE."
  (romacs-theme-roles--attributes
   (nth 1 (assq face (plist-get (romacs-theme-rows theme) :faces)))))

(defun romacs-theme-author-test--strings (tree)
  "Every string in TREE, a hash table of them."
  (let ((found (make-hash-table :test 'equal)) (pending (list tree)))
    (while pending
      (let ((each (car pending)))
        (setq pending (cdr pending))
        (cond ((stringp each) (puthash each t found))
              ((vectorp each) (setq pending (append each pending)))
              ((consp each) (setq pending (cons (car each) (cons (cdr each) pending)))))))
    found))

(romacs-deftest romacs-theme-author-helpers-are-oklab ()
  (romacs-should (equal (romacs-theme-lighten "#336699" 0.1) "#5084b9"))
  (romacs-should (equal (romacs-theme-darken "#336699" 0.1) "#14497a"))
  (romacs-should (equal (romacs-theme-blend "#000000" "#ffffff" 0.5) "#636363"))
  (romacs-should (equal (romacs-color-rotate "#336699" 29 12) "#44629b")))

(romacs-deftest romacs-theme-author-a-theme-is-a-plain-theme ()
  (romacs-should (custom-theme-p 'romacs-author-dark))
  (romacs-should (eq (plist-get (get 'romacs-author-dark 'theme-properties) :kind) 'color-scheme))
  (romacs-should (eq (plist-get (get 'romacs-author-light 'theme-properties) :background-mode) 'light))
  (romacs-should (eq (plist-get (get 'romacs-author-light 'theme-properties) :family) 'romacs-author))
  (romacs-should (eq (get 'romacs-author-dark 'romacs-style) 'flat))
  (let ((before custom-enabled-themes))
    (unwind-protect
        (progn
          (enable-theme 'romacs-author-dark)
          (romacs-should (custom-theme-enabled-p 'romacs-author-dark))
          (romacs-should (equal (face-attribute 'default :background) "#101418"))
          (romacs-should (equal (face-attribute 'default :foreground) "#d8dee9"))
          (romacs-should (equal (face-attribute 'font-lock-keyword-face :foreground) "#c678dd"))
          (romacs-should (equal (face-attribute 'region :background) (romacs-theme-blend "#101418" "#61afef" 0.3)))
          (romacs-should (stringp (face-attribute 'mode-line :background)))
          (disable-theme 'romacs-author-dark)
          (romacs-should (not (custom-theme-enabled-p 'romacs-author-dark)))
          (enable-theme 'romacs-author-dark)
          (romacs-should (equal (face-attribute 'default :background) "#101418")))
      (disable-theme 'romacs-author-dark)
      (dolist (theme (reverse before)) (enable-theme theme)))))

(romacs-deftest romacs-theme-author-a-style-is-checked ()
  (romacs-should (eq (romacs-theme--check-style 'modern) 'modern))
  (romacs-should (equal (romacs-theme--check-style '(:radius-pane 4)) '(:radius-pane 4)))
  (romacs-should-error (romacs-theme--check-style 'glossy)))

(romacs-deftest romacs-theme-author-a-row-reads-earlier-rows ()
  (romacs-should (equal (romacs-theme--eval-palette '((bg "#101418") (bg-alt (romacs-theme-lighten bg 0.05))
                                                      (other bg-alt) (third (romacs-theme-color 'other))))
                        (let ((alt (romacs-theme-lighten "#101418" 0.05)))
                          `((bg "#101418") (bg-alt ,alt) (other bg-alt) (third ,alt))))))

(romacs-deftest romacs-theme-author-eight-colours-make-a-palette ()
  (let ((resolved (plist-get (romacs-theme-rows 'romacs-author-light) :palette)))
    (dolist (entry resolved)
      (romacs-should (or (eq (car entry) 'mode) (eq (cdr entry) 'unspecified) (stringp (cdr entry)))))
    (romacs-should (eq (cdr (assq 'mode resolved)) 'light))))

(romacs-deftest romacs-theme-author-the-mode-is-inferred ()
  (romacs-should (eq (romacs-theme--infer-mode '((bg "#fafafa"))) 'light))
  (romacs-should (eq (romacs-theme--infer-mode '((bg "#101418"))) 'dark)))

(romacs-deftest romacs-theme-author-faces-win-on-any-face ()
  ;; Romacs's own rows included, and over a floor.
  (romacs-should (equal (plist-get (romacs-theme-author-test--row 'romacs-author-dark 'romacs-comment) :foreground)
                        "#e06c75"))
  ;; An override merges: what it does not name stays.
  (let ((string (romacs-theme-author-test--row 'romacs-author-dark 'font-lock-string-face)))
    (romacs-should (equal (plist-get string :box) '(:line-width 1 :color "#98c379")))
    (romacs-should (stringp (plist-get string :foreground)))))

(romacs-deftest romacs-theme-author-faces-reach-every-selectable-clause ()
  ;; A graphic display selects the first clause, not the `t' one.
  (let ((spec (romacs-theme--with-attributes '((default :inherit base) (((supports :box t)) :box 1) (t :underline t))
                                             '(:foreground "#e06c75"))))
    (romacs-should (equal spec '((default :inherit base)
                                 (((supports :box t)) :box 1 :foreground "#e06c75")
                                 (t :underline t :foreground "#e06c75")))))
  ;; A spec with no clause for every display gains one.
  (romacs-should (equal (romacs-theme--with-attributes '((default :inherit base)) '(:weight bold))
                        '((default :inherit base) (t :weight bold))))
  (romacs-should (equal (romacs-theme--with-attributes '((t (:foreground "#000000"))) '(:weight bold))
                        '((t :foreground "#000000" :weight bold)))))

(romacs-deftest romacs-theme-author-a-floor-reads-the-default-clause ()
  ;; `romacs-theme-button' keeps its colours in a `default' clause.
  (let* ((rows '((button ((default :foreground "#858585" :background "#808080")
                          (((class color)) :box 1))))))
    (romacs-should (equal (nth 3 (car (romacs-theme-roles-floors rows '((bg . "#101418"))))) "#808080")))
  (dolist (theme '(romacs-author-dark romacs-author-light))
    (let* ((attributes (romacs-theme-author-test--row theme 'romacs-theme-button))
           (fg (plist-get attributes :foreground)) (bg (plist-get attributes :background)))
      (romacs-should (and fg bg (>= (romacs-color-contrast fg bg) 4.5))))))

(romacs-deftest romacs-theme-author-rum-s-names-read-as-modus-s ()
  ;; `romacs-theme-modus-retrieve-palette-value's answers.
  (let* ((palette '((a nil) (a "#111111") (b c) (c "#222222") (d 12) (e (list)) (f unspecified) (g missing)))
         (bindings (romacs-theme--modus-bindings palette)))
    (dolist (name '(a b c d e f g))
      (romacs-should (equal (cdr (assq name bindings))
                            (romacs-theme-modus-retrieve-palette-value name palette))))))

(romacs-deftest romacs-theme-author-a-branch-follows-the-mode ()
  (romacs-should (plist-get (romacs-theme-author-test--row 'romacs-author-dark 'show-paren-match) :underline))
  (romacs-should (plist-get (romacs-theme-author-test--row 'romacs-author-light 'show-paren-match) :overline)))

(romacs-deftest romacs-theme-author-attributes-other-than-colours-stay ()
  ;; `error' is a slot as well as a face.
  (romacs-should (equal (romacs-theme-roles--override '(some-face :inherit error :foreground error) 'dark
                                                      '((error . "#ff0000")))
                        '(some-face :inherit error :foreground "#ff0000"))))

(romacs-deftest romacs-theme-author-no-modus-colour-shows ()
  ;; Faces and variables, not only the palette: distinct inputs share no
  ;; value with the vivendi core, and no colour is a literal of the table.
  (let* ((rows (romacs-theme-rows 'romacs-author-dark))
         (shown (romacs-theme-author-test--strings (list (plist-get rows :faces) (plist-get rows :variables))))
         (core (delq nil (mapcar (lambda (row) (and (stringp (nth 1 row)) (nth 1 row)))
                                 romacs-theme-modus-vivendi-palette))))
    ;; Black and white are readable foregrounds the org-habit faces compute
    ;; from their backgrounds, not the core's.
    (romacs-should (null (seq-remove (lambda (colour) (member colour '("#000000" "#ffffff")))
                                     (seq-filter (lambda (colour) (gethash colour shown)) core))))
    (dolist (row (plist-get rows :faces))
      (let ((strings (romacs-theme-author-test--strings row)))
        (when (or (gethash "#000000" strings) (gethash "#ffffff" strings))
          (romacs-should (string-prefix-p "org-habit-" (symbol-name (car row)))))))
    (romacs-should (not (gethash "gray50" shown)))))

(romacs-deftest romacs-theme-author-a-name-reaches-what-reads-it ()
  (let* ((red "#ee1122")
         (resolved (romacs-theme-roles-resolve '((mode dark) (bg "#101418") (fg "#d8dee9") (red "#ee1122") (green "#98c379")
                                                 (yellow "#e5c07b") (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2")))))
    (dolist (name '(red bg-graph-red-0 ansi-1 fg-term-red bg-term-red))
      (romacs-should (equal (cdr (assq name resolved)) red)))))

(romacs-deftest romacs-theme-author-one-authority ()
  ;; A common override moves every reader together.
  (let* ((romacs-theme-common-palette-overrides '((comment red-faint)))
         (rows (romacs-theme-roles-rows '((mode dark) (bg "#101418") (fg "#d8dee9") (red "#e06c75") (green "#98c379")
                                          (yellow "#e5c07b") (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2"))
                                        nil nil))
         (faces (plist-get rows :faces))
         (colour (cdr (assq 'comment (plist-get rows :palette))))
         (read (lambda (face) (plist-get (romacs-theme-roles--attributes (nth 1 (assq face faces))) :foreground))))
    (romacs-should (equal colour (cdr (assq 'red-faint (plist-get rows :palette)))))
    (romacs-should (equal (funcall read 'font-lock-comment-face) colour))
    (romacs-should (equal (funcall read 'romacs-comment) colour))))

(romacs-deftest romacs-theme-author-the-terminal-has-its-sixteen ()
  (let* ((palette '((mode dark) (bg "#101418") (fg "#d8dee9") (red "#e06c75") (green "#98c379") (yellow "#e5c07b")
                    (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2") (ansi-9 "#ff1234") (ansi-8 "#445566") (ansi-7 "#aabbcc")))
         (rows (romacs-theme-roles-rows palette nil nil))
         (attribute (lambda (face) (plist-get (romacs-theme-roles--attributes (nth 1 (assq face (plist-get rows :faces)))) :foreground)))
         (vector (nth 1 (assq 'ansi-color-names-vector (plist-get rows :variables)))))
    (romacs-should (equal (funcall attribute 'ansi-color-bright-red) "#ff1234"))
    (romacs-should (equal (aref vector 0) "#445566"))
    (romacs-should (equal (aref vector 7) "#aabbcc"))))

(romacs-deftest romacs-theme-author-text-reads-at-its-floor ()
  (let* ((rows (romacs-theme-rows 'romacs-author-light))
         (resolved (plist-get rows :palette)))
    ;; The pass reaches a fixed point: no pair is left to move.
    (romacs-should (null (romacs-theme-roles-floors (plist-get rows :faces) resolved)))
    (dolist (move (plist-get rows :floors))
      (romacs-should (>= (romacs-color-contrast (nth 2 move) (nth 3 move)) (nth 4 move))))))

(romacs-deftest romacs-theme-author-a-floor-reads-through-inherit ()
  (let ((rows '((parent ((t :background "#101418")))
                (child ((t :inherit parent :foreground "#202428")))
                (tint ((t :background "#303438")))
                (shown ((t :inverse-video t :foreground "#202428")))))
        (canvas '((bg . "#fafafa"))))
    (let ((moves (romacs-theme-roles-floors rows canvas)))
      (romacs-should (equal (mapcar #'car moves) '(child)))
      (romacs-should (equal (nth 3 (car moves)) "#101418")))))

(romacs-deftest romacs-theme-author-the-value-is-custom-s-source ()
  (let* ((rows (romacs-theme-rows 'romacs-author-dark))
         (settings (get 'romacs-author-dark 'theme-settings)))
    (dolist (row (plist-get rows :faces))
      ;; Custom keeps an obsolete alias under the face it names.
      (let* ((face (or (get (car row) 'face-alias) (car row)))
             (setting (seq-find (lambda (each) (and (eq (car each) 'theme-face) (eq (nth 1 each) face))) settings)))
        (romacs-should (equal (nth 3 setting) (nth 1 row)))))
    (dolist (row (plist-get rows :variables))
      (let ((setting (seq-find (lambda (each) (and (eq (car each) 'theme-value) (eq (nth 1 each) (car row)))) settings)))
        (romacs-should (equal (eval (nth 3 setting) t) (nth 1 row)))))))

(romacs-deftest romacs-theme-author-variables-belong-to-the-value ()
  (let ((one (plist-get (romacs-theme-rows 'romacs-author-dark) :variables)))
    (romacs-should (equal (assq 'romacs-author-variable one) '(romacs-author-variable "#e06c75")))
    ;; A theme's own row wins over the table's, each variable once.
    (let ((rows (romacs-theme-roles-rows (get 'romacs-author-dark 'romacs-palette) nil '((frame-background-mode 'light)))))
      (romacs-should (equal (seq-filter (lambda (row) (eq (car row) 'frame-background-mode)) (plist-get rows :variables))
                            '((frame-background-mode light)))))))

(romacs-deftest romacs-theme-author-coverage-is-the-same-for-every-theme ()
  (romacs-should (equal (romacs-theme-uncovered 'romacs-author-dark) (romacs-theme-uncovered 'romacs-author-light))))

(romacs-deftest romacs-theme-author-the-doc-s-blocks-run ()
  (with-temp-buffer
    (insert-file-contents (expand-file-name "doc/themes.org" romacs-theme-author-test--root))
    (goto-char (point-min))
    (let ((blocks 0))
      (while (re-search-forward "^#\\+begin_src emacs-lisp\n\\(\\(?:.*\n\\)*?\\)#\\+end_src" nil t)
        (setq blocks (1+ blocks))
        (let ((text (match-string 1)) (end (point)))
          (with-temp-buffer
            (insert text)
            (goto-char (point-min))
            (condition-case nil
                (while t (eval (read (current-buffer)) t))
              (end-of-file nil)))
          (goto-char end)))
      (romacs-should (> blocks 2)))))

;;; romacs-theme-author-test.el ends here
