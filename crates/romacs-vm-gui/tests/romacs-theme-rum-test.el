;;; romacs-theme-rum-test.el --- The stock theme is the configured Rum -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; In a `--loadup' session of its own (gnu_completion.rs
;; `romacs_theme_rum_tests_pass'): the comparison defines faces of
;; packages the session has not loaded and realises every face for a
;; graphic display, which no other test should see.

(require 'romacs-theme)

(defconst romacs-theme-test--fixtures
  (expand-file-name "../../romacs-face/tests/gnu/fixtures/" (file-name-directory load-file-name))
  "The GNU oracle's fixtures.")

;; The stock theme is the configured Rum look face for face (fixture
;; rum-configured.eld; tests/gnu/rum-configured.el captures it).  Both
;; sides realise their faces for a graphical truecolour display, which
;; this batch session's record is told it is for the comparison, as
;; dump.el tells GNU.  A face from a package this session has not loaded
;; is defined from GNU's defface spec; a Modus helper face is compared
;; under its Romacs name.  Families are the session's fonts
;; and heights are relative to the default face's; an unspecified weight,
;; slant or width is normal.
(defconst romacs-theme-test-rum-divergences '(font-lock-comment-delimiter-face vertical-border)
  "Faces the stock theme shows otherwise, on purpose: dividers in the
window background under pane chrome (DIVERGENCES.md VM2-PANE-CHROME);
comment delimiters inheriting the comment face, as GNU's defface has
them, so they follow it (same colour, Modus's slant through it).")

(defun romacs-theme-test--normalise (attributes default)
  (let ((base (plist-get default :height)) out)
    (while attributes
      (let ((key (pop attributes)) (value (pop attributes)))
        (when (and (eq key :height) (numberp value) (numberp base))
          (setq value (/ (round (* 100 (/ (float value) base))) 100.0)))
        (when (and (memq key '(:weight :slant :width)) (eq value 'unspecified)) (setq value 'normal))
        (when (eq key :inherit)
          (setq value (if (consp value) (mapcar #'romacs-theme-test--rename value) (romacs-theme-test--rename value))))
        (unless (or (eq key :family) (and (eq key :foundry) (member value '("default" unspecified)))
                    (and (memq key '(:weight :slant :width)) (eq value 'normal)))
          (push (cons key value) out))))
    (sort out (lambda (a b) (string< (car a) (car b))))))

(defun romacs-theme-test--rename (face)
  (if (and (symbolp face) (string-prefix-p "modus-themes-" (symbol-name face)))
      (intern (concat "romacs-theme-" (substring (symbol-name face) (length "modus-themes-"))))
    face))

(defun romacs-theme-test--attributes (face default-row)
  (let (out)
    (dolist (pair face-attribute-name-alist (nreverse out))
      (let ((value (face-attribute face (car pair) nil 'default)))
        (unless (equal value (plist-get default-row (car pair)))
          (setq out (cons value (cons (car pair) out))))))))

(defun romacs-theme-test--graphic-faces ()
  "Realise every face for the selected frame as on a truecolour display."
  (frame-set-background-mode (selected-frame) t)
  (face-set-after-frame-default (selected-frame)))

(romacs-deftest romacs-theme-the-stock-theme-is-the-configured-rum ()
  (romacs-should (equal custom-enabled-themes '(default)))
  (let* ((fixture (with-temp-buffer
                    (insert-file-contents (expand-file-name "rum-configured.eld" romacs-theme-test--fixtures))
                    (read (current-buffer))))
         (skip (plist-get fixture :init-only))
         differing)
    (dolist (row (plist-get fixture :faces))
      (let ((face (romacs-theme-test--rename (car row))))
        (unless (or (facep face) (not (eq face (car row))))
          (face-spec-set face (nth 1 row) 'face-defface-spec))))
    (let ((romacs-display (append '(:graphic t :color t :color-cells 16777216 :planes 24
                                    :visual-class true-color :display-type color :palette nil)
                                  romacs-display))
          (noninteractive nil))
      (romacs-theme-test--graphic-faces)
      (set-face-attribute 'default (selected-frame) :height (plist-get (plist-get fixture :default) :height))
      (let ((default-row (mapcan (lambda (pair) (list (car pair) (face-attribute 'default (car pair) nil 'default)))
                                 face-attribute-name-alist))
            (gnu-default (plist-get fixture :default)))
        (dolist (row (plist-get fixture :faces))
          (let ((face (romacs-theme-test--rename (car row))))
            (unless (or (memq face skip)
                        (and (facep face)
                             (equal (romacs-theme-test--normalise (nth 2 row) gnu-default)
                                    (romacs-theme-test--normalise (romacs-theme-test--attributes face default-row) default-row))))
              (push face differing))))))
    (message "romacs-theme: the configured Rum look: %d of %d faces; differing %S"
             (- (length (plist-get fixture :faces)) (length skip) (length differing))
             (- (length (plist-get fixture :faces)) (length skip)) differing)
    (romacs-should (equal (sort differing #'string<) romacs-theme-test-rum-divergences))))

(romacs-deftest romacs-theme-a-frame-s-faces-keep-the-new-frame-defaults ()
  ;; faces.el face-set-after-frame-default, which a session runs for its
  ;; window: each face from its specs, then its new-frame defaults over
  ;; them (`internal-merge-in-global-face'); an unspecified default
  ;; unspecifies the spec's value.  Pinned GNU: ("blue" bold), then
  ;; ("blue" unspecified).
  (defface romacs-theme-test-global '((t :foreground "red" :weight bold)) "A fixture face.")
  (set-face-attribute 'romacs-theme-test-global t :foreground "blue")
  (face-set-after-frame-default (selected-frame))
  (romacs-should (equal (list (face-attribute 'romacs-theme-test-global :foreground)
                              (face-attribute 'romacs-theme-test-global :weight))
                        '("blue" bold)))
  (set-face-attribute 'romacs-theme-test-global t :weight 'unspecified)
  (face-set-after-frame-default (selected-frame))
  (romacs-should (equal (list (face-attribute 'romacs-theme-test-global :foreground)
                              (face-attribute 'romacs-theme-test-global :weight))
                        '("blue" unspecified)))
  ;; A slot a frame's own setter would not take arrives as is: a :fontset
  ;; of `reset'.  Pinned GNU: (reset reset).
  (set-face-attribute 'romacs-theme-test-global t :fontset 'reset)
  (face-set-after-frame-default (selected-frame))
  (romacs-should (equal (list (internal-get-lisp-face-attribute 'romacs-theme-test-global :fontset (selected-frame))
                              (internal-get-lisp-face-attribute 'romacs-theme-test-global :fontset t))
                        '(reset reset))))

;;; romacs-theme-rum-test.el ends here
