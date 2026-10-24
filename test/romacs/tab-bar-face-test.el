;;; tab-bar-face-test.el --- Test tab-bar face attributes rendering -*- lexical-binding: t -*-

;; Test that the tab-bar correctly renders all face attributes via the
;; status-line pipeline: per-tab faces with different colors, weight,
;; italic, underline, overline, strike-through, box decorations, and
;; inline images — all visible simultaneously.
;;
;; Usage: RUST_LOG=debug ./target/release/romacs -Q -l test/romacs/tab-bar-face-test.el
;;
;; What to check:
;; - Each tab shows a different face style (all visible at once)
;; - Per-tab face colors (tab-bar-tab vs tab-bar-tab-inactive) work
;; - Image glyphs (Pik.png, Pik_left.png) appear correctly
;; - Box-faced "LISP" label with green box and bold text
;; - Active tab (click to switch) shows tab-bar-tab face
;;
;; Tab bar is rendered via the layout engine's status-line pipeline
;; (GlyphRowRole::TabBar), not the old GPU overlay.

;;; Code:

(require 'cl-lib)

;; ---- Image glyph helpers (from user config) ----
(defun tab-bar-face-test--glyph-create (path)
  "Create an image glyph from PATH."
  (create-image path nil nil :ascent 'center :height 24))

(defvar tab-bar-face-test--glyph-right
  (let ((f (expand-file-name "~/Pictures/Pik.png")))
    (when (file-exists-p f)
      (tab-bar-face-test--glyph-create f)))
  "Right skeleton glyph image (Pik.png).")

(defvar tab-bar-face-test--glyph-left
  (let ((f (expand-file-name "~/Pictures/Pik_left.png")))
    (when (file-exists-p f)
      (tab-bar-face-test--glyph-create f)))
  "Left skeleton glyph image (Pik_left.png).")

;; Each test case: (LABEL FACE-SPEC DESCRIPTION)
;; FACE-SPEC is a face plist, :image for image test, :glyph-combo for
;; the user's image+box+face combo, or nil for default.
(defvar tab-bar-face-test-cases
  `(("D"   nil                                        "Default (no extra face)")
    ("R"   (foreground-color . "red")                 "Red foreground")
    ("B"   (:foreground "white" :background "blue")   "White on blue bg")
    ("b"   (:weight bold)                             "Bold weight")
    ("i"   (:slant italic)                            "Italic slant")
    ("U"   (:underline t)                             "Underline")
    ("u"   (:underline (:color "green" :style line))  "Green underline")
    ("O"   (:overline t)                              "Overline")
    ("S"   (:strike-through t)                        "Strike-through")
    ("X"   (:box (:line-width 2 :color "green"))      "Box decoration")
    ("C"   (:weight bold :underline t
            :foreground "yellow" :background "dark green")
                                                      "Bold+underline+yellow")
    ("*"   (:weight bold :underline t :overline t
            :strike-through t :foreground "cyan"
            :background "dark red"
            :box (:line-width 1 :color "yellow"))
                                                      "All decorations")
    ("GLY" :glyph-combo                               "Image+box glyph combo")
    ("IMG" :image                                     "Image icon in tab"))
  "List of (LABEL FACE-SPEC DESCRIPTION) test cases.")

(defun tab-bar-face-test--format-tab-name (tab _i)
  "Custom tab name formatter that applies per-tab face from buffer-local var."
  (let* ((buf-name (alist-get 'name tab))
         (buf (get-buffer buf-name))
         (label (and buf (buffer-local-value 'tab-bar-face-test--label buf)))
         (face-spec (and buf (buffer-local-value 'tab-bar-face-test--face buf))))
    (unless label (setq label buf-name))
    (cond
     ;; Glyph combo: image + boxed LISP + image (user's config pattern)
     ((eq face-spec :glyph-combo)
      (concat
       " "
       (if tab-bar-face-test--glyph-right
           (propertize "  " 'display tab-bar-face-test--glyph-right)
         "[R]")
       (propertize " LISP " 'face
                   '((:box (:style nil :line-width -2 :color "green")
                      :weight bold
                      :foreground "green")))
       (propertize " Machine " 'face '(:background "green"))
       (if tab-bar-face-test--glyph-left
           (propertize "  " 'display tab-bar-face-test--glyph-left)
         "[L]")
       " "))
     ;; Image test: prepend a scaled image icon
     ((eq face-spec :image)
      (let ((img (create-image (expand-file-name "~/Pictures/4k_image_10.jpg")
                               'jpeg nil :height 16 :ascent 80)))
        (concat (propertize " " 'display img) label)))
     ;; Face spec: propertize the label
     (face-spec
      (propertize label 'face face-spec))
     ;; Default: just the label
     (t label))))

(defun tab-bar-face-test ()
  "Run the tab-bar face attribute test.
Creates one tab per test case, all visible at once."
  (tab-bar-mode 1)
  (setq tab-bar-tab-name-format-function #'tab-bar-face-test--format-tab-name)

  ;; Set per-tab faces: active vs inactive should look different
  (set-face-attribute 'tab-bar-tab nil
                      :foreground "white" :background "dark blue"
                      :weight 'bold :box '(:line-width 1 :color "steel blue"))
  (set-face-attribute 'tab-bar-tab-inactive nil
                      :foreground "gray70" :background "gray25"
                      :slant 'italic)

  ;; Create one tab per test case
  (let ((first t))
    (dolist (tc tab-bar-face-test-cases)
      (cl-destructuring-bind (label face-spec desc) tc
        (let ((buf-name (format "*tb:%s*" label)))
          (if first
              (progn
                (switch-to-buffer (get-buffer-create buf-name))
                (setq first nil))
            (tab-bar-new-tab)
            (switch-to-buffer (get-buffer-create buf-name)))
          ;; Store per-tab metadata as buffer-local vars
          (setq-local tab-bar-face-test--label label)
          (setq-local tab-bar-face-test--face face-spec)
          (erase-buffer)
          (insert (format "Tab: %s\n%s\n\nFace: %S\n" label desc face-spec))))))

  ;; Switch to first tab
  (tab-bar-select-tab 1)

  ;; Show info in the first buffer
  (let ((buf (get-buffer "*tb:D*")))
    (when buf
      (with-current-buffer buf
        (goto-char (point-max))
        (insert "\n\n--- Tab Bar Face Test ---\n")
        (insert "All test cases visible simultaneously as tabs.\n")
        (insert "Click different tabs to see active vs inactive face.\n")
        (insert "Tab bar rendered via status-line pipeline (GlyphRowRole::TabBar).\n\n")
        (insert "Test cases:\n")
        (dolist (tc tab-bar-face-test-cases)
          (cl-destructuring-bind (label _face desc) tc
            (insert (format "  [%s] %s\n" label desc))))
        (goto-char (point-min)))))

  (force-mode-line-update t)
  (redisplay t)
  (message "Tab-bar face test: %d tabs created. Click tabs to test active/inactive faces."
           (length tab-bar-face-test-cases)))

;; Run the test
(tab-bar-face-test)

;;; tab-bar-face-test.el ends here
