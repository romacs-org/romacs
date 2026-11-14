;;; romacs-theme-custom-test.el --- The stock theme, with Custom -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; lisp/romacs/romacs-theme.el's themes under GNU's Custom, in a `--loadup'
;; session, which starts in the stock theme `default'
;; (gnu_completion.rs `romacs_theme_lisp_tests_pass').  The colour engine
;; and palettes alone: test/romacs/romacs-test/romacs-theme-test.el.

(require 'romacs-theme)

(defconst romacs-theme-test--fixtures
  (expand-file-name "../../romacs-face/tests/gnu/fixtures/" (file-name-directory load-file-name))
  "The GNU oracle's fixtures.")

(romacs-deftest romacs-theme-the-face-engine-answers-to-gnu-s-names ()
  ;; GNU's names reach the recalculation in Rust, and the theme functions
  ;; are the ones whose loops are there, not custom.el's and faces.el's.
  (dolist (name '(face-spec-recalc custom-theme-recalc-face face-spec-choose
                  face-spec-set-match-display face-attr-match-p face-list))
    (romacs-should (subrp (indirect-function name))))
  (dolist (name '(enable-theme disable-theme))
    (romacs-should (string-suffix-p "romacs-custom.el" (symbol-file name 'defun))))
  (dolist (name '(face-set-after-frame-default frame-set-background-mode))
    (romacs-should (string-suffix-p "romacs-frame.el" (symbol-file name 'defun)))))

(romacs-deftest romacs-theme-an-override-chooses-by-the-flip-before-it ()
  ;; A theme's black `default' flips the background mode inside the
  ;; replay; the override spec chosen after it reads the mode as it is
  ;; then, as `face-spec-recalc' does.
  (let ((themes custom-enabled-themes))
    (unwind-protect
        (progn
          (mapc #'disable-theme custom-enabled-themes)
          (deftheme romacs-test-black "A black default.")
          (custom-theme-set-faces 'romacs-test-black '(default ((t :background "#000000"))))
          (put 'default 'face-override-spec
               '((((background dark)) :foreground "#aaffaa") (t :foreground "#005500")))
          (enable-theme 'romacs-test-black)
          (romacs-should (eq (frame-parameter nil 'background-mode) 'dark))
          (romacs-should (equal (face-attribute 'default :foreground) "#aaffaa")))
      (put 'default 'face-override-spec nil)
      (disable-theme 'romacs-test-black)
      (mapc #'enable-theme (reverse themes)))))

(romacs-deftest romacs-theme-the-modus-generator-is-modus-s ()
  ;; lisp/romacs/romacs-theme-modus.el's generator, on the stock theme's
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
        (romacs-should (not (equal (face-attribute 'font-lock-keyword-face :foreground nil t) "#c9a2f2")))
        (romacs-should (not (equal (face-attribute 'default :background nil t) "#000000"))))
    (enable-theme 'default))
  (romacs-should (equal (face-attribute 'font-lock-keyword-face :foreground nil t) "#c9a2f2")))

(romacs-deftest romacs-theme-a-colour-scheme-replaces-the-stock-theme-once ()
  (unwind-protect
      (let ((romacs-theme--stock t))
        (romacs-theme-define 'romacs-theme-test-scheme "A fixture scheme." :mode 'dark :family 'romacs-theme-test-scheme
                             :palette romacs-theme-test-dark)
        (enable-theme 'romacs-theme-test-scheme)
        (romacs-should (equal custom-enabled-themes '(romacs-theme-test-scheme)))
        (romacs-should (equal (face-attribute 'romacs-layer-glow :foreground nil t) "#7aa2f7"))
        ;; Romacs's colour schemes are one look each: the stock theme
        ;; enabled again by hand replaces the scheme, and stays.
        (enable-theme 'default)
        (romacs-should (equal custom-enabled-themes '(default)))
        (romacs-should (null romacs-theme--stock)))
    (mapc #'disable-theme custom-enabled-themes)
    (enable-theme 'default)))

(romacs-deftest romacs-theme-rum-and-default-toggle ()
  ;; `rum' (Rum exactly) and `default' (the stock look, which starts from
  ;; it) replace each other however often they are switched, disabled and
  ;; enabled again; each switch leaves the faces a fresh enable gives.
  (unwind-protect
      (let ((look (lambda () (mapcar (lambda (face) (list face (face-attribute face :foreground nil t)
                                                          (face-attribute face :background nil t)))
                                     '(default region mode-line font-lock-keyword-face link)))))
        (load-theme 'rum t)
        (romacs-should (equal custom-enabled-themes '(rum)))
        (let ((rum (funcall look)))
          (dotimes (_ 3)
            (enable-theme 'default)
            (romacs-should (equal custom-enabled-themes '(default)))
            (enable-theme 'rum)
            (romacs-should (equal custom-enabled-themes '(rum)))
            (romacs-should (equal (funcall look) rum)))
          (disable-theme 'rum)
          (romacs-should (null custom-enabled-themes))
          (enable-theme 'rum)
          (romacs-should (equal (funcall look) rum))))
    (mapc #'disable-theme custom-enabled-themes)
    (enable-theme 'default))
  (romacs-should (equal custom-enabled-themes '(default))))

(romacs-deftest romacs-theme-a-third-party-scheme-stacks-over-rum ()
  ;; Only Romacs's own colour schemes replace each other: a third-party
  ;; one enabled over an explicitly chosen `rum' stacks, as GNU's themes do.
  (unwind-protect
      (progn
        (load-theme 'rum t)
        (custom-declare-theme 'romacs-theme-test-third nil)
        (custom-theme-set-faces 'romacs-theme-test-third '(default ((t :background "#101010" :foreground "#e0e0e0"))))
        (enable-theme 'romacs-theme-test-third)
        (romacs-should (equal custom-enabled-themes '(romacs-theme-test-third rum))))
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

(romacs-deftest romacs-theme-every-shipped-theme-has-no-fringe-strip ()
  ;; Fringes, window dividers and the vertical border are the window
  ;; background in every theme Romacs ships, so no strip shows beside a
  ;; window, as the reference session's invisible dividers do.
  (dolist (theme (mapcar #'car romacs-theme-bundled))
    (load-theme theme t t)
    (let* ((spec (lambda (face)
                   (nth 3 (car (seq-filter (lambda (s) (and (eq (car s) 'theme-face) (eq (nth 1 s) face)))
                                           (get theme 'theme-settings))))))
           (attr (lambda (face key) (plist-get (cdr (car (funcall spec face))) key)))
           (bg (funcall attr 'default :background)))
      (dolist (face '(fringe window-divider window-divider-first-pixel window-divider-last-pixel vertical-border))
        (romacs-should (equal (list theme face (funcall attr face :background)) (list theme face bg)))))))

(romacs-deftest romacs-theme-the-stock-theme-is-romacs-s-own ()
  ;; `default' grows from Rum, which stays exact.
  (load-theme 'rum t t)
  (let ((face (lambda (theme name key)
                (plist-get (romacs-theme-roles--attributes
                            (nth 1 (assq name (plist-get (romacs-theme-rows theme) :faces))))
                           key))))
    ;; Code: structure carries the colour, a variable's uses stay text.
    (romacs-should (equal (funcall face 'default 'font-lock-variable-use-face :foreground)
                          (funcall face 'default 'default :foreground)))
    (romacs-should (not (equal (funcall face 'default 'font-lock-number-face :foreground)
                               (funcall face 'default 'default :foreground))))
    (romacs-should (> (romacs-color-contrast (funcall face 'default 'font-lock-keyword-face :foreground) "#000000")
                      (romacs-color-contrast (funcall face 'default 'font-lock-type-face :foreground) "#000000")))
    ;; Secondary text reads, and line numbers sit below it.
    (romacs-should (>= (romacs-color-contrast (funcall face 'default 'font-lock-comment-face :foreground) "#000000") 7.0))
    (romacs-should (< (romacs-color-contrast (funcall face 'default 'line-number :foreground) "#000000")
                      (romacs-color-contrast (funcall face 'default 'font-lock-comment-face :foreground) "#000000")))
    ;; The bar is quiet, its dim segments read, purple is the pill's alone.
    (let ((bar (funcall face 'default 'mode-line :background)))
      (romacs-should (equal bar "#1b1814"))
      (romacs-should (stringp (funcall face 'default 'mode-line-active :overline)))
      ;; On the active line alone: `header-line' and others inherit `mode-line'.
      (romacs-should (null (funcall face 'default 'mode-line :overline)))
      (romacs-should (>= (romacs-color-contrast (funcall face 'default 'romacs-mode-line-dim :foreground) bar) 4.5)))
    (romacs-should (equal (funcall face 'default 'romacs-mode-line-pill :background) "#50269a"))
    ;; Rum keeps its bar and its code colours.
    (romacs-should (equal (funcall face 'rum 'mode-line :background) "#50269a"))
    (romacs-should (equal (funcall face 'rum 'font-lock-variable-name-face :foreground) "#00a6a6"))
    (romacs-should (null (plist-get (romacs-theme-rows 'rum) :floors)))))

(defun romacs-theme-test--pill-contrast ()
  "The status bar pill's text against its background, as shown."
  (romacs-color-contrast (romacs-chrome--hex (face-foreground 'romacs-mode-line-pill nil t))
                         (romacs-chrome--hex (face-background 'romacs-mode-line-pill nil t))))

(romacs-deftest romacs-theme-the-pill-reads-in-every-shipped-theme ()
  (require 'romacs-mode-line)
  (let ((enabled custom-enabled-themes) (romacs-theme--stock nil))
    (unwind-protect
        (dolist (entry romacs-theme-bundled)
          (mapc #'disable-theme (copy-sequence custom-enabled-themes))
          (load-theme (car entry) t)
          (romacs-should (equal (list (car entry) (>= (romacs-theme-test--pill-contrast) 3.0)) (list (car entry) t))))
      (mapc #'disable-theme (copy-sequence custom-enabled-themes))
      (dolist (theme (reverse enabled)) (enable-theme theme)))))

(romacs-deftest romacs-theme-a-pill-and-its-text-come-from-one-theme ()
  ;; A theme that sets only the bar, over the theme that set the pill.
  (require 'romacs-mode-line)
  (let ((enabled custom-enabled-themes) (romacs-theme--stock nil))
    (unwind-protect
        (progn
          (mapc #'disable-theme (copy-sequence custom-enabled-themes))
          (load-theme 'default t)
          (deftheme romacs-pill-test)
          (custom-theme-set-faces 'romacs-pill-test
                                  '(mode-line ((t :background "#e8e8e8" :foreground "#202020")))
                                  '(mode-line-active ((t :background "#e8e8e8" :foreground "#202020"))))
          (enable-theme 'romacs-pill-test)
          (romacs-should (equal (face-background 'romacs-mode-line-pill nil t) "#50269a"))
          (romacs-should (equal (face-foreground 'romacs-mode-line-pill nil t) "#d4c5a0"))
          (romacs-should (>= (romacs-theme-test--pill-contrast) 4.5))
          (disable-theme 'romacs-pill-test)
          (romacs-should (equal (face-background 'romacs-mode-line-pill nil t) "#50269a")))
      (mapc #'disable-theme (copy-sequence custom-enabled-themes))
      (dolist (theme (reverse enabled)) (enable-theme theme)))))

(romacs-deftest romacs-theme-a-pill-keeps-text-its-theme-inherits ()
  ;; The foreground comes through `:inherit', not from the theme's own row.
  (require 'romacs-mode-line)
  (let ((enabled custom-enabled-themes) (romacs-theme--stock nil))
    (unwind-protect
        (progn
          (mapc #'disable-theme (copy-sequence custom-enabled-themes))
          (defface romacs-pill-test-parent '((t :foreground "#fefefe")) "A parent.")
          (deftheme romacs-pill-inherit-test)
          (custom-theme-set-faces 'romacs-pill-inherit-test
                                  '(mode-line-active ((t :background "#202020" :foreground "#eeeeee")))
                                  '(romacs-mode-line-pill ((t :inherit romacs-pill-test-parent))))
          (enable-theme 'romacs-pill-inherit-test)
          (romacs-should (equal (face-foreground 'romacs-mode-line-pill nil t) "#fefefe"))
          (romacs-should (eq (face-attribute 'romacs-mode-line-pill :foreground) 'unspecified))
          (disable-theme 'romacs-pill-inherit-test)
          (romacs-should (equal (face-foreground 'romacs-mode-line-pill nil t)
                                (face-foreground 'mode-line-active nil t))))
      (mapc #'disable-theme (copy-sequence custom-enabled-themes))
      (dolist (theme (reverse enabled)) (enable-theme theme)))))

(romacs-deftest romacs-theme-a-pill-outlasts-no-theme ()
  ;; What the pill was given with no theme enabled stays out of the theme's way
  ;; on a frame made later as on this one.
  (require 'romacs-mode-line)
  (let ((enabled custom-enabled-themes) (romacs-theme--stock nil))
    (unwind-protect
        (progn
          (mapc #'disable-theme (copy-sequence custom-enabled-themes))
          (romacs-should (eq (face-attribute 'romacs-mode-line-pill :background t) 'unspecified))
          (load-theme 'default t)
          (romacs-should (eq (face-attribute 'romacs-mode-line-pill :background t) 'unspecified)))
      (mapc #'disable-theme (copy-sequence custom-enabled-themes))
      (dolist (theme (reverse enabled)) (enable-theme theme)))))

(romacs-deftest romacs-theme-a-hand-set-pill-outlives-a-theme ()
  ;; The theme gives the colour the user set, so only the user's stays after.
  (require 'romacs-mode-line)
  (let ((enabled custom-enabled-themes) (romacs-theme--stock nil))
    (unwind-protect
        (progn
          (mapc #'disable-theme (copy-sequence custom-enabled-themes))
          (set-face-attribute 'romacs-mode-line-pill nil :background "#abcdef")
          (romacs-mode-line-pill-faces)
          (romacs-should (equal (face-background 'romacs-mode-line-pill nil t) "#abcdef"))
          (deftheme romacs-pill-same-test)
          (custom-theme-set-faces 'romacs-pill-same-test
                                  '(romacs-mode-line-pill ((t :background "#abcdef"))))
          (enable-theme 'romacs-pill-same-test)
          (disable-theme 'romacs-pill-same-test)
          (romacs-should (equal (face-background 'romacs-mode-line-pill nil t) "#abcdef")))
      (set-face-attribute 'romacs-mode-line-pill nil :background 'unspecified)
      (mapc #'disable-theme (copy-sequence custom-enabled-themes))
      (dolist (theme (reverse enabled)) (enable-theme theme)))))

(romacs-deftest romacs-theme-a-pill-face-keeps-what-is-not-its-colour ()
  ;; A hand-set weight through a theme that only moves the bar; the state's
  ;; declared bold through a new colour.
  (require 'romacs-mode-line)
  (let ((enabled custom-enabled-themes) (romacs-theme--stock nil))
    (unwind-protect
        (progn
          (mapc #'disable-theme (copy-sequence custom-enabled-themes))
          (romacs-should (eq (face-attribute 'romacs-mode-line-state :inherit) 'bold))
          (set-face-attribute 'romacs-mode-line-pill nil :weight 'bold)
          (deftheme romacs-pill-bar-test)
          (custom-theme-set-faces 'romacs-pill-bar-test
                                  '(mode-line-active ((t :background "#123456" :foreground "#fefefe"))))
          (enable-theme 'romacs-pill-bar-test)
          (romacs-should (eq (face-attribute 'romacs-mode-line-pill :weight) 'bold))
          (disable-theme 'romacs-pill-bar-test)
          (romacs-should (eq (face-attribute 'romacs-mode-line-pill :weight) 'bold))
          (romacs-should (eq (face-attribute 'romacs-mode-line-state :inherit) 'bold)))
      (set-face-attribute 'romacs-mode-line-pill nil :weight 'unspecified)
      (mapc #'disable-theme (copy-sequence custom-enabled-themes))
      (dolist (theme (reverse enabled)) (enable-theme theme)))))

(romacs-deftest romacs-style-a-theme-declares-its-own ()
  (let ((enabled custom-enabled-themes) (romacs-style-overrides nil))
    (unwind-protect
        (let ((tokens (lambda () (romacs-style-resolve))))
          (mapc #'disable-theme custom-enabled-themes)
          ;; Nothing declares `modern' yet beyond flat's tokens.
          (romacs-should (equal (funcall tokens) (romacs-style--over romacs-style-flat romacs-style-modern)))
          (load-theme 'rum t)
          (romacs-should (eq (romacs-style--scheme) 'rum))
          (romacs-should (equal (funcall tokens) romacs-style-flat))
          ;; A theme's plist, then the user's, each over the last.
          (deftheme romacs-style-test)
          (custom-theme-set-faces 'romacs-style-test '(default ((t :background "#101010"))))
          (put 'romacs-style-test 'romacs-style '(:radius-band 9 :radius-layer 3))
          (enable-theme 'romacs-style-test)
          (romacs-should (eq (romacs-style--scheme) 'romacs-style-test))
          (romacs-should (eql (plist-get (funcall tokens) :radius-band) 9))
          (let ((romacs-style-overrides '(:radius-band 2)))
            (romacs-should (eql (plist-get (funcall tokens) :radius-band) 2))
            (romacs-should (eql (plist-get (funcall tokens) :radius-layer) 3)))
          ;; Disabling it gives the style back.
          (disable-theme 'romacs-style-test)
          (romacs-should (eql (plist-get (funcall tokens) :radius-band) 4)))
      (mapc #'disable-theme custom-enabled-themes)
      (dolist (theme (reverse enabled)) (enable-theme theme)))))

(romacs-deftest romacs-style-a-user-s-option-wins ()
  (let ((style (romacs-style--over romacs-style-flat '(:radius-pane 10))))
    (let ((romacs-pane-radius 6))
      (romacs-should (eql (romacs-style--option 'romacs-pane-radius :radius-pane style) 10)))
    (let ((romacs-pane-radius 3))
      (romacs-should (eql (romacs-style--option 'romacs-pane-radius :radius-pane style) 3)))))

(romacs-deftest romacs-style-the-wire-is-numbers ()
  (romacs-should (equal (romacs-style--wire romacs-style-flat) [8 4 18 6 0.55 14 0.35])))

(romacs-deftest romacs-style-classic-chrome-is-flat ()
  ;; No theme's or user's tokens reach GNU's look.
  (let ((romacs-style--resolved (romacs-style--over romacs-style-flat '(:radius-layer 17 :radius-band 9))))
    (romacs-should (equal (cdr (assq 'romacs-style (romacs-chrome--parameters t))) (romacs-style--wire romacs-style-flat)))
    (romacs-should (equal (aref (cdr (assq 'romacs-style (romacs-chrome--parameters nil))) 0) 17))))

(romacs-deftest romacs-theme-gnu-s-themes-are-not-offered ()
  ;; Romacs offers its own colour schemes.  GNU's are a package away, and
  ;; `newcomers-presets', which sets options rather than colours, stays.
  (let ((available (custom-available-themes)))
    (dolist (theme '(adwaita deeper-blue dichromacy leuven leuven-dark light-blue manoj-dark misterioso
                     tango tango-dark tsdh-dark tsdh-light wheatgrass whiteboard wombat
                     modus-operandi modus-operandi-tinted modus-operandi-deuteranopia modus-operandi-tritanopia
                     modus-vivendi modus-vivendi-tinted modus-vivendi-deuteranopia modus-vivendi-tritanopia))
      (romacs-should (equal (list theme (memq theme available)) (list theme nil))))
    (dolist (theme '(default rum tokyonight-night catppuccin-latte newcomers-presets))
      (romacs-should (memq theme available))))
  (romacs-should (eq (condition-case nil (progn (load-theme 'tango-dark t t) 'loaded) (error 'refused)) 'refused)))

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

;; romacs-theme-modus.el adapts etc/modus/modus-themes.el as it loads.  A
;; newer Modus changes what it defines with no diff of its own: these say
;; what it takes and name what changed.

(defconst romacs-theme-test--modus-taken
  '((custom-declare-face-loop)
    ("defface" "modus-themes-bold") ("defface" "modus-themes-slant")
    ("defface" "modus-themes-fixed-pitch") ("defface" "modus-themes-ui-variable-pitch")
    ("defface" "modus-themes-reset-soft") ("defface" "modus-themes-button")
    ("defcustom" "modus-themes-italic-constructs") ("defcustom" "modus-themes-bold-constructs")
    ("defcustom" "modus-themes-variable-pitch-ui") ("defcustom" "modus-themes-mixed-fonts")
    ("defconst" "modus-themes--weight-widget") ("defconst" "modus-themes--headings-widget")
    ("defcustom" "modus-themes-headings") ("defcustom" "modus-themes-common-palette-user")
    ("defcustom" "modus-themes-common-palette-overrides") ("defconst" "modus-themes-common-palette-mappings")
    ("defconst" "modus-themes-vivendi-palette") ("defvar" "modus-themes--hex-regexp")
    ("defun" "modus-themes--color-hex-p") ("defun" "modus-themes--hex-or-name-to-rgb")
    ("defun" "modus-themes--wcag-contribution") ("defun" "modus-themes-wcag-formula")
    ("defun" "modus-themes-contrast") ("defun" "modus-themes--color-eight-to-six-digits")
    ("defun" "modus-themes-adjust-value") ("defun" "modus-themes-retrieve-palette-value")
    ("defun" "modus-themes-color-dark-p") ("defun" "modus-themes-get-readable-foreground")
    ("defun" "modus-themes--property-lookup") ("defun" "modus-themes--bold-weight")
    ("defun" "modus-themes--slant") ("defun" "modus-themes--fixed-pitch")
    ("defun" "modus-themes--variable-pitch-ui") ("defconst" "modus-themes-weights")
    ("defun" "modus-themes--weight") ("defun" "modus-themes--heading") ("defun" "modus-themes--box")
    ("defconst" "modus-themes-faces") ("defconst" "modus-themes-custom-variables")
    ("defun" "modus-themes-blend") ("defun" "modus-themes-generate-color-blend")
    ("defun" "modus-themes-generate-color-warmer") ("defun" "modus-themes-generate-color-cooler")
    ("defun" "modus-themes-color-warm-p") ("defun" "modus-themes-color-is-warm-or-cool-p")
    ("defun" "modus-themes-generate-color-warmer-or-cooler") ("defun" "modus-themes-generate-palette"))
  "The Modus forms romacs-theme-modus.el takes from Modus 5.3.0, in order.")

(romacs-deftest romacs-theme-modus-takes-the-forms-of-the-version-it-names ()
  (require 'romacs-theme-modus)
  (let ((header (with-temp-buffer
                  (insert-file-contents (locate-library "romacs-theme-modus.el"))
                  (and (re-search-forward "^;; Adapted from modus-themes\\.el \\([0-9.]+\\)" nil t)
                       (match-string 1))))
        (source (with-temp-buffer
                  (insert-file-contents romacs-theme-modus-source)
                  (and (re-search-forward "^;; Version: \\([0-9.]+\\)" nil t) (match-string 1)))))
    (romacs-should (equal header source)))
  (let ((case-fold-search nil) (parse-sexp-ignore-comments t))
    (romacs-should (equal (mapcar #'car (romacs-theme-modus--taken
                                         (romacs-theme-modus--top-level
                                          (with-temp-buffer
                                            (insert-file-contents romacs-theme-modus-source)
                                            (buffer-string)))))
                          romacs-theme-test--modus-taken))))

(romacs-deftest romacs-theme-modus-defines-under-romacs-names-with-the-session-s-defaults ()
  (require 'romacs-theme-modus)
  ;; Modus's own names all go; the package's name stays, in the options'
  ;; `:package-version'.
  (let ((left nil))
    (dolist (form (romacs-theme-modus-forms))
      (dolist (atom (flatten-tree form))
        (when (and (symbolp atom) (string-prefix-p "modus-themes-" (symbol-name atom)))
          (push atom left))))
    (romacs-should (null left)))
  (romacs-should (equal (mapcar #'default-value '(romacs-theme-italic-constructs romacs-theme-bold-constructs
                                                  romacs-theme-mixed-fonts romacs-theme-variable-pitch-ui))
                        '(nil nil nil nil)))
  (romacs-should (equal (default-value 'romacs-theme-headings)
                        '((1 . (ultrabold 1.35)) (2 . (semibold 1.2)) (agenda-date . (1.2))
                          (agenda-structure . (variable-pitch light 1.8)) (t . (1.15)))))
  ;; A caller names the core palette, and the active window's line is
  ;; `mode-line' itself.
  (romacs-should (equal (condition-case err (progn (romacs-theme-modus-generate-palette '((bg-main "#000000") (fg-main "#ffffff")) 'warm) nil)
                          (error (cadr err)))
                        "A core palette is required"))
  (romacs-should (member (read "`(mode-line-active ((,c :inherit mode-line)))") romacs-theme-modus-faces)))

(romacs-deftest romacs-theme-modus-scans-modus-as-written-whatever-the-session-s-settings ()
  (require 'romacs-theme-modus)
  ;; A comment in a form is skipped, its parenthesis too, and names match
  ;; case-sensitively, though the default settings say otherwise.
  (with-temp-buffer
    (let ((parse-sexp-ignore-comments nil) (case-fold-search t))
      (romacs-should
       (equal (mapcar (lambda (entry) (cons (car entry) (cddr entry)))
                      (romacs-theme-modus--top-level
                       "(defun modus-themes-a ()\n  ;; (unbalanced\n  1)\n(defvar Modus-themes-b nil)\n"))
              '((("defun" "modus-themes-a") . "(defun modus-themes-a ()\n  ;; (unbalanced\n  1)")
                (("defvar" "Modus-themes-b") . "(defvar Modus-themes-b nil)"))))
      (romacs-should (equal (romacs-theme-modus--adapt '("defvar" "x") "(defvar x '(Modus-themes-b modus-themes-b))")
                            '(defvar x '(Modus-themes-b romacs-theme-modus-b)))))))

;;; romacs-theme-custom-test.el ends here
