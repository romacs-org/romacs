;;; romacs-theme-custom-test.el --- The stock theme, with Custom -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-theme.el's themes under GNU's Custom, in a `--loadup'
;; session, which starts in the stock theme `default'
;; (gnu_completion.rs `romacs_theme_lisp_tests_pass').  The colour engine
;; and palettes alone: test/romacs-lisp/romacs-theme-test.el.

(require 'romacs-theme)

(defconst romacs-theme-test--fixtures
  (expand-file-name "../../romacs-face/tests/gnu/fixtures/" (file-name-directory load-file-name))
  "The GNU oracle's fixtures.")

(romacs-deftest romacs-theme-the-modus-generator-is-modus-s ()
  ;; rlisp/romacs-theme-modus.el's generator, on the stock theme's
  ;; reference palette, answers as GNU with modus-themes 5.3.0 does, entry
  ;; for entry (fixture rum-generator.eld).
  (require 'romacs-theme-modus)
  (let* ((fixture (with-temp-buffer
                    (insert-file-contents (expand-file-name "rum-generator.eld" romacs-theme-test--fixtures))
                    (read (current-buffer))))
         (ours (romacs-theme-modus-generate-palette (plist-get fixture :base) 'warm
                                                    romacs-theme-modus-vivendi-palette))
         (theirs (plist-get fixture :palette)))
    (romacs-should (= (length ours) (length theirs)))
    (romacs-should (null (seq-remove (lambda (entry) (equal entry (assq (car entry) theirs))) ours)))))

(defconst romacs-theme-test-dark
  '((mode dark) (bg "#1a1b26") (bg-dark "#16161e") (bg-darker "#101014") (bg-alt "#24283b")
    (fg "#c0caf5") (fg-dim "#565f89") (fg-light "#a9b1d6") (fg-bright "#ffffff")
    (accent "#7aa2f7") (selection "#283457") (muted "#444b6a")
    (red "#f7768e") (yellow "#e0af68") (green "#9ece6a") (cyan "#7dcfff") (blue "#7aa2f7")
    (magenta "#bb9af7") (bright-red "#ff899d") (bright-yellow "#faba4a") (bright-green "#9fe044")
    (bright-cyan "#a4daff") (bright-blue "#8db0ff") (bright-magenta "#c7a9ff"))
  "A dark fixture palette.")

(romacs-deftest romacs-theme-the-stock-theme-text-reads ()
  (let* ((resolved (romacs-theme-palette 'default)) (bg (cdr (assq 'bg resolved)))
         (dim '(shadow line-number mode-line-inactive font-lock-comment-face romacs-comment completions-annotations org-done)))
    (dolist (row (romacs-theme-faces resolved))
      (let* ((attributes (cdr (car (car (cdr row)))))
             (fg (plist-get attributes :foreground)) (on (or (plist-get attributes :background) bg)))
        (when (and (stringp fg) (not (string-prefix-p "ansi-color-" (symbol-name (car row))))
                   (not (memq (car row) '(fringe window-divider window-divider-first-pixel window-divider-last-pixel
                                         show-paren-mismatch romacs-pane-focus romacs-pane-border romacs-layer-glow
                                         romacs-landing-mark romacs-landing-word vertical-border))))
          (romacs-should (>= (romacs-color-contrast fg on) (if (memq (car row) dim) 3.0 4.5))))))))

(defconst romacs-theme-test-rum
  '((default "#d4c5a0" "#000000") (cursor nil "#9099d9") (mode-line "#d4c5a0" "#50269a")
    (mode-line-inactive "#756b59" "#141414") (region nil "#3e4451") (highlight nil "#7030af")
    (minibuffer-prompt "#d2b580" nil) (font-lock-keyword-face "#b395db" nil)
    (font-lock-string-face "#aacb75" nil) (font-lock-comment-face "#756b59" nil)
    (font-lock-function-name-face "#c28fea" nil) (font-lock-variable-name-face "#00a6a6" nil)
    (font-lock-constant-face "#72b6e4" nil) (font-lock-type-face "#00b8ad" nil)
    (font-lock-builtin-face "#d0a3e8" nil) (font-lock-warning-face "#e89e30" nil)
    (link "#4cc9f0" nil) (error "#c97054" nil) (warning "#e89e30" nil) (success "#00b8ad" nil)
    (romacs-comment "#756b59" nil) (romacs-code "#aacb75" nil) (romacs-result "#72b6e4" nil)
    (fringe "#000000" "#000000") (window-divider "#000000" "#000000")
    (window-divider-first-pixel "#000000" "#000000") (window-divider-last-pixel "#000000" "#000000")
    (show-paren-match nil "#7030af") (romacs-layer-glow "#d48a20" nil))
  "Rum's look, which the stock theme keeps: (FACE FOREGROUND BACKGROUND).")

(romacs-deftest romacs-theme-the-stock-theme-is-rum-s-look ()
  (romacs-should (equal custom-enabled-themes '(default)))
  (require 'romacs-faces)
  (dolist (row romacs-theme-test-rum)
    (let ((face (nth 0 row)))
      (when (nth 1 row) (romacs-should (equal (list face (face-attribute face :foreground nil t)) (list face (nth 1 row)))))
      (when (nth 2 row) (romacs-should (equal (list face (face-attribute face :background nil t)) (list face (nth 2 row))))))))

(defun romacs-theme-test--look ()
  (mapcar (lambda (face) (list face (face-attribute face :foreground nil t) (face-attribute face :background nil t)))
          '(default font-lock-keyword-face mode-line region link)))

(romacs-deftest romacs-theme-defining-a-theme-changes-no-face ()
  (let ((before (romacs-theme-test--look)) (enabled custom-enabled-themes))
    (load-theme 'default t t)
    (romacs-theme-define 'romacs-theme-test-fixture "A fixture." :mode 'dark :family 'romacs-theme-test-fixture
                         :palette romacs-theme-test-dark)
    (romacs-should (equal (romacs-theme-test--look) before))
    (romacs-should (equal custom-enabled-themes enabled))))

(romacs-deftest romacs-theme-disabling-the-stock-theme-gives-gnu-s-faces ()
  (unwind-protect
      (progn
        (disable-theme 'default)
        (romacs-should (null custom-enabled-themes))
        (romacs-should (not (equal (face-attribute 'font-lock-keyword-face :foreground nil t) "#b395db")))
        (romacs-should (not (equal (face-attribute 'default :background nil t) "#000000"))))
    (enable-theme 'default))
  (romacs-should (equal (face-attribute 'font-lock-keyword-face :foreground nil t) "#b395db")))

(romacs-deftest romacs-theme-a-colour-scheme-replaces-the-stock-theme-once ()
  (unwind-protect
      (let ((romacs-theme--stock t))
        (romacs-theme-define 'romacs-theme-test-scheme "A fixture scheme." :mode 'dark :family 'romacs-theme-test-scheme
                             :palette romacs-theme-test-dark)
        (enable-theme 'romacs-theme-test-scheme)
        (romacs-should (equal custom-enabled-themes '(romacs-theme-test-scheme)))
        (romacs-should (equal (face-attribute 'romacs-layer-glow :foreground nil t) "#7aa2f7"))
        ;; The stock theme enabled again by hand is a choice: it stays.
        (enable-theme 'default)
        (romacs-should (equal custom-enabled-themes '(default romacs-theme-test-scheme)))
        (romacs-should (null romacs-theme--stock)))
    (mapc #'disable-theme custom-enabled-themes)
    (enable-theme 'default)))

(romacs-deftest romacs-theme-a-package-s-theme-keeps-the-stock-theme ()
  ;; A theme a package keeps its own settings in, such as `use-package''s for
  ;; `:custom-face', is no colour scheme: the stock theme and the default
  ;; face's font stay.  A theme that sets `default' without saying what it
  ;; is replaces the stock theme, as most third-party themes need.
  (unwind-protect
      (let ((romacs-theme--stock t)
            (font (list (face-attribute 'default :family) (face-attribute 'default :height))))
        (custom-declare-theme 'romacs-theme-test-settings nil)
        (custom-theme-set-faces 'romacs-theme-test-settings '(romacs-theme-test-face ((t :weight bold))))
        (enable-theme 'romacs-theme-test-settings)
        (require 'use-package-core)
        (defface romacs-theme-test-late '((t :foreground "red")) "A face defined late.")
        (romacs-should (memq 'default custom-enabled-themes))
        (romacs-should (equal (list (face-attribute 'default :family) (face-attribute 'default :height)) font))
        (custom-declare-theme 'romacs-theme-test-doom nil)
        (custom-theme-set-faces 'romacs-theme-test-doom '(default ((t :background "#101010" :foreground "#e0e0e0"))))
        (enable-theme 'romacs-theme-test-doom)
        (romacs-should (not (memq 'default custom-enabled-themes))))
    (mapc #'disable-theme custom-enabled-themes)
    (enable-theme 'default)))

(romacs-deftest romacs-theme-the-stock-theme-merged-pairs-read ()
  (let* ((resolved (romacs-theme-palette 'default)) (faces (romacs-theme-faces resolved))
         (fg (lambda (face) (plist-get (cdr (car (car (cdr (assq face faces))))) :foreground)))
         (slot (lambda (name) (cdr (assq name resolved)))))
    (dolist (bg (list (funcall slot 'glass) (funcall slot 'selection)))
      (romacs-should (>= (romacs-color-contrast (funcall fg 'romacs-complete-match) bg) 4.5)))
    (dolist (face '(romacs-mode-line-error romacs-mode-line-warning romacs-mode-line-ok romacs-mode-line-state))
      (romacs-should (>= (romacs-color-contrast (funcall fg face) (funcall slot 'bg-mode-line)) 4.5)))
    ;; Headings are recorded with Rum's heights.
    (romacs-should (equal (plist-get (cdr (car (car (cdr (assq 'org-level-1 faces))))) :height) 1.35))
    (romacs-should (assq 'default (get 'org-level-1 'theme-face)))))

(romacs-deftest romacs-theme-reports-the-active-palette ()
  (romacs-should (equal (cdr (assq 'accent (romacs-theme-palette))) "#d48a20"))
  (romacs-should (equal (cdr (assq 'glow (romacs-theme-palette 'default))) "#d48a20")))

(romacs-deftest romacs-theme-the-bundled-themes-are-available ()
  (let ((available (custom-available-themes)))
    (dolist (entry romacs-theme-bundled)
      (romacs-should (memq (car entry) available)))
    (romacs-should (memq 'system available))))

(romacs-deftest romacs-theme-every-shipped-theme-loads-and-reads ()
  (let ((enabled custom-enabled-themes))
    (dolist (entry romacs-theme-bundled)
      (let ((theme (car entry)))
        (load-theme theme t t)
        (romacs-should (custom-theme-p theme))
        (romacs-should (eq (plist-get (get theme 'theme-properties) :background-mode) (nth 1 entry)))
        (let* ((resolved (romacs-theme-palette theme)) (bg (cdr (assq 'bg resolved)))
               (faces (romacs-theme-faces resolved))
               (fg (lambda (face) (plist-get (cdr (car (car (cdr (assq face faces))))) :foreground))))
          (romacs-should (eq (cdr (assq 'mode resolved)) (nth 1 entry)))
          (dolist (face '(default font-lock-keyword-face font-lock-string-face link error minibuffer-prompt))
            (romacs-should (>= (romacs-color-contrast (funcall fg face) bg) 4.5)))
          (romacs-should (>= (romacs-color-contrast (funcall fg 'font-lock-comment-face) bg) 3.0))
          (dolist (on (list (cdr (assq 'glass resolved)) (cdr (assq 'selection resolved))))
            (romacs-should (>= (romacs-color-contrast (funcall fg 'romacs-complete-match) on) 4.5))))))
    (romacs-should (equal custom-enabled-themes enabled))))

(romacs-deftest romacs-theme-a-variant-is-found-before-it-is-loaded ()
  (unwind-protect
      (let ((romacs-theme--stock t))
        (load-theme 'omarchy-catppuccin t)
        (romacs-should (equal custom-enabled-themes '(omarchy-catppuccin)))
        (theme-choose-variant t)
        (romacs-should (equal custom-enabled-themes '(omarchy-catppuccin-latte)))
        (romacs-should (eq (frame-parameter nil 'background-mode) (if (display-graphic-p) 'light (frame-parameter nil 'background-mode)))))
    (mapc #'disable-theme custom-enabled-themes)
    (enable-theme 'default)))

(romacs-deftest romacs-theme-system-needs-its-file ()
  (let ((romacs-theme-system-file nil))
    (romacs-should (eq (condition-case nil (progn (load-theme 'system t t) 'loaded) (user-error 'refused)) 'refused))
    (romacs-should (not (custom-theme-p 'system)))))

(romacs-deftest romacs-theme-system-is-its-file-s-palette ()
  (let* ((file (make-temp-file "romacs-system-" nil ".toml"))
         (romacs-theme-system-file file))
    (unwind-protect
        (progn
          (with-temp-file file
            (insert "mode = \"light\"\naccent = \"#1e66f5\"\nbackground = \"#eff1f5\"\nforeground = \"#4c4f69\"\nhyprland_active_border = \"rgba(1e66f5ee)\"\n"))
          (load-theme 'system t t)
          (let ((resolved (romacs-theme-palette 'system)))
            (romacs-should (eq (cdr (assq 'mode resolved)) 'light))
            (romacs-should (equal (cdr (assq 'accent resolved)) "#1e66f5"))
            (romacs-should (equal (cdr (assq 'bg resolved)) "#eff1f5"))
            ;; Keys the file leaves out are the stock theme's.
            (romacs-should (equal (cdr (assq 'green resolved)) "#9cbd6f"))))
      (delete-file file))))

(romacs-deftest romacs-theme-a-light-system-palette-reads ()
  ;; Catppuccin Latte's full palette: derived slots are its own, not the
  ;; stock theme's dark pins.
  (let* ((file (make-temp-file "romacs-system-" nil ".toml"))
         (romacs-theme-system-file file))
    (unwind-protect
        (progn
          (with-temp-file file
            (insert "mode = \"light\"\naccent = \"#1e66f5\"\nselection = \"#ccd0da\"\nmuted = \"#acb0be\"
background = \"#eff1f5\"\ndark_background = \"#e3e4e8\"\ndarker_background = \"#d7d8dc\"
lighter_background = \"#dce0e8\"\nforeground = \"#4c4f69\"\ndark_foreground = \"#9ca0b0\"
light_foreground = \"#5c5f77\"\nbright_foreground = \"#4c4f69\"\nred = \"#d20f39\"\nyellow = \"#df8e1d\"
orange = \"#fe640b\"\ngreen = \"#40a02b\"\ncyan = \"#179299\"\nblue = \"#1e66f5\"\nmagenta = \"#ea76cb\"
brown = \"#dc8a78\"\nbright_red = \"#d20f39\"\nbright_yellow = \"#df8e1d\"\nbright_green = \"#40a02b\"
bright_cyan = \"#179299\"\nbright_blue = \"#1e66f5\"\nbright_magenta = \"#ea76cb\"\n"))
          (load-theme 'system t t)
          (let* ((resolved (romacs-theme-palette 'system)) (faces (romacs-theme-faces resolved))
                 (match (plist-get (cdr (car (car (cdr (assq 'romacs-complete-match faces))))) :foreground)))
            (romacs-should (not (equal (cdr (assq 'glass resolved)) "#11100d")))
            (romacs-should (not (equal (cdr (assq 'bg-region resolved)) "#3e4451")))
            (dolist (on (list (cdr (assq 'glass resolved)) (cdr (assq 'selection resolved))))
              (romacs-should (>= (romacs-color-contrast match on) 4.5)))))
      (delete-file file))))

;;; romacs-theme-custom-test.el ends here
