;;; romacs-theme-custom-test.el --- The stock theme, with Custom -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-theme.el's themes under GNU's Custom, in a `--loadup'
;; session, which starts in the stock theme `default'
;; (gnu_completion.rs `romacs_theme_lisp_tests_pass').  The colour engine
;; and palettes alone: test/romacs-lisp/romacs-theme-test.el.

(require 'romacs-theme)

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

;;; romacs-theme-custom-test.el ends here
