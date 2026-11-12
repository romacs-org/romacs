;;; romacs-theme-test.el --- Palettes, derived slots and faces -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; lisp/romacs/romacs-theme.el's colours, palettes and faces, in the machine's own
;; prelude without GNU's Lisp, where themes are declared as the faces' own
;; specs.  With Custom (the stock theme enabled, replaced, disabled):
;; crates/romacs-vm-gui/tests/romacs-theme-custom-test.el.

(require 'romacs-theme)

(defconst romacs-theme-test-dark
  '((mode dark) (bg "#1a1b26") (bg-dark "#16161e") (bg-darker "#101014") (bg-alt "#24283b")
    (fg "#c0caf5") (fg-dim "#565f89") (fg-light "#a9b1d6") (fg-bright "#ffffff")
    (accent "#7aa2f7") (selection "#283457") (muted "#444b6a")
    (red "#f7768e") (yellow "#e0af68") (green "#9ece6a") (cyan "#7dcfff") (blue "#7aa2f7")
    (magenta "#bb9af7") (bright-red "#ff899d") (bright-yellow "#faba4a") (bright-green "#9fe044")
    (bright-cyan "#a4daff") (bright-blue "#8db0ff") (bright-magenta "#c7a9ff"))
  "A dark palette without `orange' and `brown'.")

(defconst romacs-theme-test-light
  '((mode light) (bg "#eff1f5") (bg-dark "#e6e9ef") (bg-darker "#dce0e8") (bg-alt "#ccd0da")
    (fg "#4c4f69") (fg-dim "#8c8fa1") (fg-light "#5c5f77") (fg-bright "#1e1e2e")
    (accent "#1e66f5") (selection "#bcc0cc") (muted "#bcc0cc")
    (red "#d20f39") (yellow "#df8e1d") (orange "#fe640b") (green "#40a02b") (cyan "#179299")
    (blue "#1e66f5") (magenta "#8839ef") (brown "#dd7878") (bright-red "#de293e")
    (bright-yellow "#eea02d") (bright-green "#49af3d") (bright-cyan "#209fb5")
    (bright-blue "#456eff") (bright-magenta "#fe85d8"))
  "A light palette whose muted colour is too faint to read on its background.")

(defun romacs-theme-test--hex-p (value)
  (and (stringp value) (string-match-p "\\`#[0-9a-f]\\{6\\}\\'" value)))

(defun romacs-theme-test--channels (hex)
  "HEX, \"#rrggbb\", as (R G B), each from 0.0 to 1.0."
  (mapcar (lambda (at) (/ (string-to-number (substring hex at (+ at 2)) 16) 255.0)) '(1 3 5)))

(romacs-deftest romacs-theme-colour-functions-give-color-el-s-values ()
  ;; Each computed by pinned GNU's color.el, which these match bit for bit.
  (romacs-should (equal (romacs-color-mix "#000000" "#ffffff" 0.5) "#636363"))
  (romacs-should (equal (romacs-color-mix "#d48a20" "#3548cf" 0.25) "#aa826a"))
  (romacs-should (equal (romacs-color-lift "#3548cf" 0.2) "#708eff"))
  (romacs-should (equal (romacs-color-lift "#ff0000" 0.3) "#fedfda"))
  (romacs-should (equal (romacs-color-chroma "#d48a20" 0.6) "#bf9361"))
  (romacs-should (equal (romacs-color-floor "#333333" "#000000" 4.5) "#757575"))
  (romacs-should (equal (romacs-color-floor "#cccccc" "#ffffff" 4.5) "#767676"))
  (romacs-should (= (romacs-color-contrast "#ffffff" "#000000") 21.0)))

(romacs-deftest romacs-theme-srgb-round-trips-within-a-step ()
  (dolist (hex '("#000000" "#ffffff" "#d48a20" "#7030af" "#9cbd6f"))
    (let ((back (romacs-theme-test--channels (romacs-color-from-oklab (romacs-color-oklab hex))))
          (there (romacs-theme-test--channels hex)))
      (romacs-should (< (apply #'max (mapcar #'abs (list (- (nth 0 back) (nth 0 there)) (- (nth 1 back) (nth 1 there))
                                                         (- (nth 2 back) (nth 2 there)))))
                        (/ 1.5 255))))))

(romacs-deftest romacs-theme-lift-keeps-hue-and-stays-in-gamut ()
  (let ((before (romacs-color--lch "#3548cf")) (after (romacs-color--lch (romacs-color-lift "#3548cf" 0.2))))
    (romacs-should (> (nth 0 after) (nth 0 before)))
    (romacs-should (< (abs (- (nth 2 after) (nth 2 before))) 0.05)))
  (romacs-should (romacs-color--in-gamut (romacs-color-oklab (romacs-color-lift "#ff0000" 0.3)))))

(romacs-deftest romacs-theme-floor-moves-only-a-failing-foreground ()
  (romacs-should (equal (romacs-color-floor "#d4c5a0" "#000000" 4.5) "#d4c5a0"))
  (romacs-should (>= (romacs-color-contrast (romacs-color-floor "#333333" "#000000" 4.5) "#000000") 4.5))
  (let ((dark-on-light (romacs-color-floor "#cccccc" "#ffffff" 4.5)))
    (romacs-should (< (nth 0 (romacs-color-oklab dark-on-light)) (nth 0 (romacs-color-oklab "#cccccc"))))))

(romacs-deftest romacs-theme-floor-reaches-its-minimum-from-either-side ()
  ;; Black on black moves up from its endpoint; #888888 on #777777 cannot
  ;; reach 4.5 going up (white is 4.48) and does going down.
  (romacs-should (>= (romacs-color-contrast (romacs-color-floor "#000000" "#000000" 4.5) "#000000") 4.5))
  (romacs-should (>= (romacs-color-contrast (romacs-color-floor "#888888" "#777777" 4.5) "#777777") 4.5))
  ;; Two backgrounds judged at once: a colour that fits one and not the
  ;; other is no answer (white fits both).
  (let ((both (romacs-color-floor-on "#737373" '("#020202" "#767676") 4.5)))
    (romacs-should (>= (romacs-color-contrast both "#020202") 4.5))
    (romacs-should (>= (romacs-color-contrast both "#767676") 4.5)))
  ;; On black and white the colours that fit are a band narrower than a
  ;; step: #757575 is one.
  (let ((band (romacs-color-floor-on "#747474" '("#000000" "#ffffff") 4.5)))
    (romacs-should (>= (romacs-color-contrast band "#000000") 4.5))
    (romacs-should (>= (romacs-color-contrast band "#ffffff") 4.5)))
  ;; A blue accent on black and on near-white: the band that fits lies
  ;; between two of the scan's samples.
  (let ((band (romacs-color-floor-on "#7aa2f7" '("#000000" "#fbfbfb") 4.5)))
    (romacs-should (>= (romacs-color-contrast band "#000000") 4.5))
    (romacs-should (>= (romacs-color-contrast band "#fbfbfb") 4.5)))
  ;; The light fixture's bright red there: a band one 8-bit step wide.
  (let ((band (romacs-color-floor-on "#de293e" '("#000000" "#fbfbfb") 4.5)))
    (romacs-should (>= (romacs-color-contrast band "#000000") 4.5))
    (romacs-should (>= (romacs-color-contrast band "#fbfbfb") 4.5)))
  (let ((both (romacs-color-floor-on "#7aa2f7" '("#283457" "#1f2335") 4.5)))
    (romacs-should (>= (romacs-color-contrast both "#283457") 4.5))
    (romacs-should (>= (romacs-color-contrast both "#1f2335") 4.5))))

(defun romacs-theme-test--fg (faces face)
  (plist-get (cdr (car (car (cdr (assq face faces))))) :foreground))

(defun romacs-theme-test--attribute (faces face attribute)
  (plist-get (cdr (car (car (cdr (assq face faces))))) attribute))

(romacs-deftest romacs-theme-text-reads-on-every-background-it-shows-on ()
  (dolist (palette (list romacs-theme-test-dark romacs-theme-test-light))
    (let* ((resolved (romacs-theme-resolve palette)) (faces (romacs-theme-faces resolved))
           (slot (lambda (name) (cdr (assq name resolved)))))
      ;; Completion matches on the panel and on its selected row.
      (dolist (bg (list (funcall slot 'glass) (funcall slot 'selection)))
        (romacs-should (>= (romacs-color-contrast (romacs-theme-test--fg faces 'romacs-complete-match) bg) 4.5)))
      (romacs-should (>= (romacs-color-contrast (romacs-theme-test--fg faces 'completions-common-part) (funcall slot 'selection)) 4.5))
      ;; Keys: bold, fixed-pitch, no background or box, readable in help
      ;; (the default background), on popups and on the completion layer.
      (romacs-should (equal (romacs-theme-test--attribute faces 'help-key-binding :inherit) '(bold fixed-pitch)))
      (dolist (attribute '(:background :box))
        (romacs-should (not (plist-member (cdr (car (car (cdr (assq 'help-key-binding faces))))) attribute))))
      (dolist (bg (list (funcall slot 'bg) (funcall slot 'bg-alt) (funcall slot 'glass)))
        (romacs-should (>= (romacs-color-contrast (romacs-theme-test--fg faces 'help-key-binding) bg) 4.5)))
      ;; The mode line's states on its band.
      (dolist (face '(romacs-mode-line-error romacs-mode-line-warning romacs-mode-line-ok romacs-mode-line-state
                      romacs-mode-line-dim romacs-mode-line-bright))
        (romacs-should (>= (romacs-color-contrast (romacs-theme-test--fg faces face) (funcall slot 'bg-mode-line)) 4.5))))))

(romacs-deftest romacs-theme-headings-have-rum-s-heights-and-weights ()
  (let ((faces (romacs-theme-faces (romacs-theme-resolve romacs-theme-test-dark))))
    (romacs-should (equal (romacs-theme-test--attribute faces 'org-level-1 :height) 1.35))
    (romacs-should (equal (romacs-theme-test--attribute faces 'org-level-2 :height) 1.2))
    (romacs-should (equal (romacs-theme-test--attribute faces 'org-level-8 :height) 1.15))
    (dolist (row '((1 ultrabold 1.35) (2 semibold 1.2) (3 bold 1.15) (8 bold 1.15)))
      (dolist (prefix '("outline-" "org-level-"))
        (let ((face (intern (format "%s%d" prefix (car row)))))
          (romacs-should (eq (romacs-theme-test--attribute faces face :weight) (nth 1 row)))
          (romacs-should (equal (romacs-theme-test--attribute faces face :height) (nth 2 row))))))
    (romacs-should (romacs-theme-test--fg faces 'outline-4))))

(romacs-deftest romacs-theme-every-slot-resolves-for-a-dark-and-a-light-palette ()
  (dolist (palette (list romacs-theme-test-dark romacs-theme-test-light))
    (let ((resolved (romacs-theme-resolve palette)))
      (dolist (slot romacs-theme-slots)
        (let ((value (cdr (assq slot resolved))))
          (romacs-should (if (eq slot 'mode) (memq value '(dark light)) (romacs-theme-test--hex-p value)))))
      ;; Derived slots read text at their floors.
      (romacs-should (>= (romacs-color-contrast (cdr (assq 'fg-muted resolved)) (cdr (assq 'bg resolved))) 3.0))
      (romacs-should (>= (romacs-color-contrast (cdr (assq 'fg-dim* resolved)) (cdr (assq 'bg resolved))) 4.5)))))

(romacs-deftest romacs-theme-lift-follows-the-palette-s-mode ()
  (let ((dark (romacs-theme-resolve romacs-theme-test-dark))
        (light (romacs-theme-resolve romacs-theme-test-light))
        (l (lambda (palette slot) (nth 0 (romacs-color-oklab (cdr (assq slot palette)))))))
    (romacs-should (> (funcall l dark 'hover) (funcall l dark 'bg-alt)))
    (romacs-should (< (funcall l light 'hover) (funcall l light 'bg-alt)))
    ;; A palette without them gets `orange' and `brown'.
    (romacs-should (romacs-theme-test--hex-p (cdr (assq 'orange dark))))
    (romacs-should (romacs-theme-test--hex-p (cdr (assq 'brown dark))))))

(romacs-deftest romacs-theme-a-pinned-slot-wins-and-a-cycle-is-refused ()
  (let ((pinned (romacs-theme-resolve (cons '(keyword "#123456") romacs-theme-test-dark))))
    (romacs-should (equal (cdr (assq 'keyword pinned)) "#123456"))
    (romacs-should (equal (cdr (assq 'glow pinned)) "#7aa2f7")))
  (romacs-should-error (romacs-theme-resolve (append '((glow accent) (accent glow)) (cdr romacs-theme-test-dark)))))

(defconst romacs-theme-test-coverage
  '(default cursor region highlight hl-line fringe window-divider vertical-border minibuffer-prompt
    mode-line mode-line-inactive mode-line-buffer-id header-line line-number line-number-current-line
    show-paren-match isearch lazy-highlight match error warning success link shadow
    completions-common-part font-lock-keyword-face font-lock-function-name-face font-lock-string-face
    font-lock-comment-face font-lock-type-face font-lock-constant-face font-lock-builtin-face
    romacs-pane-focus romacs-pane-border romacs-layer-glow romacs-layer-glass romacs-layer-toast
    romacs-layer-selection romacs-landing-mark romacs-landing-word romacs-complete-current
    romacs-complete-match romacs-complete-prompt romacs-comment romacs-code romacs-result
    romacs-mode-line-dim romacs-mode-line-error ansi-color-black ansi-color-bright-white diff-added diff-removed
    dired-directory org-block org-level-1 org-level-8 outline-1)
  "Faces every theme sets.")

(romacs-deftest romacs-theme-every-covered-face-has-a-row ()
  (dolist (palette (list romacs-theme-test-dark romacs-theme-test-light))
    (let ((faces (mapcar #'car (romacs-theme-faces (romacs-theme-resolve palette)))))
      (dolist (face romacs-theme-test-coverage)
        (romacs-should (memq face faces))))))

(defconst romacs-theme-test-dim
  '(shadow line-number mode-line-inactive font-lock-comment-face romacs-comment completions-annotations org-done)
  "Faces whose text is de-emphasised: floored at 3:1, not 4.5:1.")

(romacs-deftest romacs-theme-text-reads-on-its-background ()
  (dolist (palette (list romacs-theme-test-dark romacs-theme-test-light))
    (let* ((resolved (romacs-theme-resolve palette)) (bg (cdr (assq 'bg resolved))))
      (dolist (row (romacs-theme-faces resolved))
        (let* ((attributes (cdr (car (car (cdr row)))))
               (fg (plist-get attributes :foreground))
               (on (or (plist-get attributes :background) bg)))
          (when (and (romacs-theme-test--hex-p fg) (not (equal (substring (symbol-name (car row)) 0 (min 11 (length (symbol-name (car row))))) "ansi-color-"))
                     (not (memq (car row) '(fringe window-divider window-divider-first-pixel window-divider-last-pixel
                                           show-paren-mismatch romacs-pane-focus romacs-pane-border romacs-layer-glow
                                           romacs-landing-mark romacs-landing-word vertical-border))))
            (romacs-should (>= (romacs-color-contrast fg on) (if (memq (car row) romacs-theme-test-dim) 3.0 4.5)))))))))

;;; romacs-theme-test.el ends here
