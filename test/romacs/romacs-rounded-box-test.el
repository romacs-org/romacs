;;; romacs-rounded-box-test.el --- Test rounded corner box rendering -*- lexical-binding: t -*-

;; Test the :corner-radius box attribute for romacs GPU renderer.
;; Usage: ./target/release/romacs -Q -l test/romacs/romacs-rounded-box-test.el

;;; Commentary:

;; This test visually compares:
;;   1. Normal text (no box)
;;   2. Standard sharp box (:box with no corner-radius)
;;   3. Rounded corner box (:box with :corner-radius)
;;   4. Various corner radii and line widths

;;; Code:

;; --- Normal text (no box) ---
(defface romacs-rbox-normal
  '((t :foreground "white" :background "gray25"))
  "Normal text - no box.")

;; --- Standard sharp boxes ---
(defface romacs-rbox-sharp-thin
  '((t :foreground "white" :background "dark blue"
       :box (:line-width 1 :color "cyan")))
  "Sharp box, 1px cyan.")

(defface romacs-rbox-sharp-medium
  '((t :foreground "white" :background "dark blue"
       :box (:line-width 2 :color "cyan")))
  "Sharp box, 2px cyan.")

(defface romacs-rbox-sharp-thick
  '((t :foreground "white" :background "dark blue"
       :box (:line-width 3 :color "cyan")))
  "Sharp box, 3px cyan.")

;; --- Rounded boxes with various radii ---
(defface romacs-rbox-round-r4
  '((t :foreground "white" :background "dark green"
       :box (:line-width 2 :color "orange" :corner-radius 4)))
  "Rounded box, radius=4.")

(defface romacs-rbox-round-r8
  '((t :foreground "white" :background "dark green"
       :box (:line-width 2 :color "orange" :corner-radius 8)))
  "Rounded box, radius=8.")

(defface romacs-rbox-round-r12
  '((t :foreground "white" :background "dark green"
       :box (:line-width 2 :color "orange" :corner-radius 12)))
  "Rounded box, radius=12.")

(defface romacs-rbox-round-r16
  '((t :foreground "white" :background "dark green"
       :box (:line-width 2 :color "orange" :corner-radius 16)))
  "Rounded box, radius=16 (large).")

;; --- Rounded boxes with various line widths ---
(defface romacs-rbox-round-w1
  '((t :foreground "white" :background "purple4"
       :box (:line-width 1 :color "yellow" :corner-radius 8)))
  "Rounded box, width=1, radius=8.")

(defface romacs-rbox-round-w3
  '((t :foreground "white" :background "purple4"
       :box (:line-width 3 :color "yellow" :corner-radius 8)))
  "Rounded box, width=3, radius=8.")

(defface romacs-rbox-round-w5
  '((t :foreground "white" :background "purple4"
       :box (:line-width 5 :color "yellow" :corner-radius 8)))
  "Rounded box, width=5, radius=8.")

;; --- Rounded boxes with different colors ---
(defface romacs-rbox-red
  '((t :foreground "white" :background "gray20"
       :box (:line-width 2 :color "red" :corner-radius 8)))
  "Red rounded box.")

(defface romacs-rbox-green
  '((t :foreground "white" :background "gray20"
       :box (:line-width 2 :color "lime green" :corner-radius 8)))
  "Green rounded box.")

(defface romacs-rbox-blue
  '((t :foreground "white" :background "gray20"
       :box (:line-width 2 :color "dodger blue" :corner-radius 8)))
  "Blue rounded box.")

(defface romacs-rbox-gold
  '((t :foreground "black" :background "gold"
       :box (:line-width 3 :color "dark orange" :corner-radius 10)))
  "Gold tag style.")

;; --- Adjacent non-rounded (sharp) boxes with different colors ---
(defface romacs-rbox-adj-sharp-red
  '((t :foreground "white" :background "gray20"
       :box (:line-width 2 :color "red")))
  "Sharp red box for adjacency test.")

(defface romacs-rbox-adj-sharp-green
  '((t :foreground "white" :background "gray20"
       :box (:line-width 2 :color "lime green")))
  "Sharp green box for adjacency test.")

(defface romacs-rbox-adj-sharp-blue
  '((t :foreground "white" :background "gray20"
       :box (:line-width 2 :color "dodger blue")))
  "Sharp blue box for adjacency test.")

(defface romacs-rbox-adj-sharp-gold
  '((t :foreground "black" :background "gold"
       :box (:line-width 2 :color "dark orange")))
  "Sharp gold box for adjacency test.")

;; --- Fancy border styles ---
(defface romacs-rbox-style-solid
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "dodger blue" :corner-radius 10
             :border-style solid :border-speed 100)))
  "Fancy: solid.")

(defface romacs-rbox-style-rainbow
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "white" :corner-radius 12
             :border-style rainbow :border-speed 100)))
  "Fancy: rainbow.")

(defface romacs-rbox-style-animated-rainbow
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "white" :corner-radius 12
             :border-style animated-rainbow :border-speed 100)))
  "Fancy: animated rainbow.")

(defface romacs-rbox-style-gradient
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "deep pink" :corner-radius 10
             :border-style gradient :border-speed 100 :color2 "cyan")))
  "Fancy: gradient.")

(defface romacs-rbox-style-glow
  '((t :foreground "white" :background "gray20"
       :box (:line-width 2 :color "lime green" :corner-radius 10
             :border-style glow :border-speed 100)))
  "Fancy: pulsing glow.")

(defface romacs-rbox-style-neon
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "magenta" :corner-radius 10
             :border-style neon :border-speed 100 :color2 "cyan")))
  "Fancy: neon double-stroke.")

(defface romacs-rbox-style-dashed
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "gold" :corner-radius 10
             :border-style dashed :border-speed 100)))
  "Fancy: dashed.")

(defface romacs-rbox-style-comet
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "orange red" :corner-radius 12
             :border-style comet :border-speed 100)))
  "Fancy: comet trail.")

(defface romacs-rbox-style-iridescent
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "steel blue" :corner-radius 10
             :border-style iridescent :border-speed 100)))
  "Fancy: iridescent.")

(defface romacs-rbox-style-fire
  '((t :foreground "white" :background "gray20"
       :box (:line-width 3 :color "dark red" :corner-radius 10
             :border-style fire :border-speed 100)))
  "Fancy: fire / plasma.")

(defface romacs-rbox-style-heartbeat
  '((t :foreground "white" :background "gray20"
       :box (:line-width 2 :color "red" :corner-radius 10
             :border-style heartbeat :border-speed 100 :color2 "green")))
  "Fancy: heartbeat (red → green on beat).")

;; --- Comparison: sharp vs rounded side by side ---
(defface romacs-rbox-compare-sharp
  '((t :foreground "white" :background "#404060"
       :box (:line-width 2 :color "#8888ff")))
  "Sharp comparison box.")

(defface romacs-rbox-compare-round
  '((t :foreground "white" :background "#404060"
       :box (:line-width 2 :color "#8888ff" :corner-radius 8)))
  "Rounded comparison box.")

;; ============================================================================
;; Build the test buffer
;; ============================================================================

(defun romacs-rbox--insert-section (title)
  "Insert a section TITLE header."
  (insert "\n")
  (let ((start (point)))
    (insert (format "=== %s ===\n" title))
    (put-text-property start (point) 'face '(:weight bold :height 1.2 :foreground "gold"))))

(defun romacs-rbox--insert-sample (label face-name)
  "Insert a labeled sample with FACE-NAME applied to sample text."
  (insert (format "  %-40s " label))
  (let ((start (point)))
    (insert " Sample Text ABCdef 123 ")
    (put-text-property start (point) 'face face-name))
  (insert "\n"))

(defun romacs-rbox--insert-inline (label face-name)
  "Insert inline sample showing face in context."
  (insert (format "  %-40s before " label))
  (let ((start (point)))
    (insert "BOXED")
    (put-text-property start (point) 'face face-name))
  (insert " after\n"))

(defun romacs-rounded-box-test ()
  "Create rounded corner box test buffer."
  (interactive)
  (let ((buf (get-buffer-create "*Rounded Box Test*")))
    (switch-to-buffer buf)
    (erase-buffer)

    ;; Header
    (let ((start (point)))
      (insert "ROMACS ROUNDED CORNER BOX TEST\n")
      (put-text-property start (point) 'face '(:weight bold :height 1.8 :foreground "cyan")))
    (insert (format "Emacs version: %s | Window system: %s\n" emacs-version window-system))
    (insert (make-string 78 ?-) "\n")

    ;; === NO BOX (baseline) ===
    (romacs-rbox--insert-section "BASELINE: NO BOX")
    (romacs-rbox--insert-sample "Normal text (no box):" 'romacs-rbox-normal)

    ;; === SHARP BOX (standard Emacs) ===
    (romacs-rbox--insert-section "SHARP BOX (standard, no corner-radius)")
    (romacs-rbox--insert-sample "Sharp 1px cyan:" 'romacs-rbox-sharp-thin)
    (romacs-rbox--insert-sample "Sharp 2px cyan:" 'romacs-rbox-sharp-medium)
    (romacs-rbox--insert-sample "Sharp 3px cyan:" 'romacs-rbox-sharp-thick)

    ;; === ROUNDED BOX: varying radius ===
    (romacs-rbox--insert-section "ROUNDED BOX: VARYING CORNER RADIUS")
    (romacs-rbox--insert-sample "Rounded radius=4:" 'romacs-rbox-round-r4)
    (romacs-rbox--insert-sample "Rounded radius=8:" 'romacs-rbox-round-r8)
    (romacs-rbox--insert-sample "Rounded radius=12:" 'romacs-rbox-round-r12)
    (romacs-rbox--insert-sample "Rounded radius=16:" 'romacs-rbox-round-r16)

    ;; === ROUNDED BOX: varying line width ===
    (romacs-rbox--insert-section "ROUNDED BOX: VARYING LINE WIDTH")
    (romacs-rbox--insert-sample "Width=1 radius=8:" 'romacs-rbox-round-w1)
    (romacs-rbox--insert-sample "Width=3 radius=8:" 'romacs-rbox-round-w3)
    (romacs-rbox--insert-sample "Width=5 radius=8:" 'romacs-rbox-round-w5)

    ;; === ROUNDED BOX: colors ===
    (romacs-rbox--insert-section "ROUNDED BOX: DIFFERENT COLORS")
    (romacs-rbox--insert-sample "Red border:" 'romacs-rbox-red)
    (romacs-rbox--insert-sample "Green border:" 'romacs-rbox-green)
    (romacs-rbox--insert-sample "Blue border:" 'romacs-rbox-blue)
    (romacs-rbox--insert-sample "Gold tag style:" 'romacs-rbox-gold)

    ;; === SIDE BY SIDE COMPARISON ===
    (romacs-rbox--insert-section "SIDE BY SIDE: SHARP vs ROUNDED")
    (insert "  ")
    (let ((s (point)))
      (insert " SHARP ")
      (put-text-property s (point) 'face 'romacs-rbox-compare-sharp))
    (insert "  ")
    (let ((s (point)))
      (insert " ROUNDED ")
      (put-text-property s (point) 'face 'romacs-rbox-compare-round))
    (insert "  ")
    (let ((s (point)))
      (insert " SHARP ")
      (put-text-property s (point) 'face 'romacs-rbox-compare-sharp))
    (insert "  ")
    (let ((s (point)))
      (insert " ROUNDED ")
      (put-text-property s (point) 'face 'romacs-rbox-compare-round))
    (insert "\n")

    ;; === INLINE CONTEXT ===
    (romacs-rbox--insert-section "INLINE: BOX IN CONTEXT")
    (romacs-rbox--insert-inline "Sharp inline:" 'romacs-rbox-sharp-medium)
    (romacs-rbox--insert-inline "Rounded inline:" 'romacs-rbox-round-r8)
    (romacs-rbox--insert-inline "Gold tag inline:" 'romacs-rbox-gold)

    ;; === ADJACENT ROUNDED BOXES ===
    (romacs-rbox--insert-section "ADJACENT ROUNDED BOXES")
    (insert "  ")
    (let ((s (point)))
      (insert " Red ")
      (put-text-property s (point) 'face 'romacs-rbox-red))
    (let ((s (point)))
      (insert " Green ")
      (put-text-property s (point) 'face 'romacs-rbox-green))
    (let ((s (point)))
      (insert " Blue ")
      (put-text-property s (point) 'face 'romacs-rbox-blue))
    (let ((s (point)))
      (insert " Gold ")
      (put-text-property s (point) 'face 'romacs-rbox-gold))
    (insert "\n")

    ;; === ADJACENT NON-ROUNDED (SHARP) BOXES ===
    (romacs-rbox--insert-section "ADJACENT SHARP BOXES (no corner-radius)")
    (insert "  ")
    (let ((s (point)))
      (insert " Red ")
      (put-text-property s (point) 'face 'romacs-rbox-adj-sharp-red))
    (let ((s (point)))
      (insert " Green ")
      (put-text-property s (point) 'face 'romacs-rbox-adj-sharp-green))
    (let ((s (point)))
      (insert " Blue ")
      (put-text-property s (point) 'face 'romacs-rbox-adj-sharp-blue))
    (let ((s (point)))
      (insert " Gold ")
      (put-text-property s (point) 'face 'romacs-rbox-adj-sharp-gold))
    (insert "\n")

    ;; === FANCY BORDER STYLES ===
    (romacs-rbox--insert-section "FANCY BORDER STYLES (:border-style)")
    (romacs-rbox--insert-sample "solid:" 'romacs-rbox-style-solid)
    (romacs-rbox--insert-sample "rainbow:" 'romacs-rbox-style-rainbow)
    (romacs-rbox--insert-sample "animated-rainbow:" 'romacs-rbox-style-animated-rainbow)
    (romacs-rbox--insert-sample "gradient (pink→cyan):" 'romacs-rbox-style-gradient)
    (romacs-rbox--insert-sample "glow:" 'romacs-rbox-style-glow)
    (romacs-rbox--insert-sample "neon (magenta+cyan):" 'romacs-rbox-style-neon)
    (romacs-rbox--insert-sample "dashed:" 'romacs-rbox-style-dashed)
    (romacs-rbox--insert-sample "comet:" 'romacs-rbox-style-comet)
    (romacs-rbox--insert-sample "iridescent:" 'romacs-rbox-style-iridescent)
    (romacs-rbox--insert-sample "fire:" 'romacs-rbox-style-fire)
    (romacs-rbox--insert-sample "heartbeat:" 'romacs-rbox-style-heartbeat)

    ;; === FANCY STYLES INLINE ===
    (romacs-rbox--insert-section "FANCY STYLES: INLINE CONTEXT")
    (romacs-rbox--insert-inline "animated-rainbow inline:" 'romacs-rbox-style-animated-rainbow)
    (romacs-rbox--insert-inline "gradient inline:" 'romacs-rbox-style-gradient)
    (romacs-rbox--insert-inline "neon inline:" 'romacs-rbox-style-neon)
    (romacs-rbox--insert-inline "comet inline:" 'romacs-rbox-style-comet)
    (romacs-rbox--insert-inline "heartbeat inline:" 'romacs-rbox-style-heartbeat)

    ;; === ADJACENT FANCY BOXES ===
    (romacs-rbox--insert-section "ADJACENT FANCY BOXES")
    (insert "  ")
    (let ((s (point)))
      (insert " Rainbow ")
      (put-text-property s (point) 'face 'romacs-rbox-style-rainbow))
    (let ((s (point)))
      (insert " Gradient ")
      (put-text-property s (point) 'face 'romacs-rbox-style-gradient))
    (let ((s (point)))
      (insert " Neon ")
      (put-text-property s (point) 'face 'romacs-rbox-style-neon))
    (let ((s (point)))
      (insert " Fire ")
      (put-text-property s (point) 'face 'romacs-rbox-style-fire))
    (let ((s (point)))
      (insert " Heartbeat ")
      (put-text-property s (point) 'face 'romacs-rbox-style-heartbeat))
    (insert "\n")

    ;; === EXPECTED BEHAVIOR ===
    (romacs-rbox--insert-section "EXPECTED BEHAVIOR")
    (insert "
  - Normal text: plain colored background, no border
  - Sharp box: rectangular border with 90-degree corners
  - Rounded box: border with smooth rounded corners
  - Background fill should also be rounded (not sharp rect)
  - Border should render on top of text (visible ring)
  - Consecutive boxed chars should merge into single box span
  - Adjacent boxes with different faces should be separate
  - Corner radius is clamped to 45%% of box dimensions
")

    (goto-char (point-min))
    (setq buffer-read-only t)
    (message "Rounded box test loaded! Scroll through to check rendering.")))

;; Run automatically
(romacs-rounded-box-test)

;;; romacs-rounded-box-test.el ends here
