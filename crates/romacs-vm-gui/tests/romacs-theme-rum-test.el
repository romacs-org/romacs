;;; romacs-theme-rum-test.el --- The stock theme is the configured Rum -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; In a `--loadup' session of its own (gnu_completion.rs
;; `romacs_theme_rum_tests_pass'): the comparison defines faces of
;; packages the session has not loaded and realises every face for a
;; graphic display, which no other test should see.

(require 'romacs-theme)

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
