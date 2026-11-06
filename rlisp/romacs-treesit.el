;;; romacs-treesit.el --- Tree-sitter around visiting a file  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A visit's first parse starts as soon as the file's text is in, on a
;; thread of its own, and runs while `after-find-file' chooses and starts
;; the major mode.  The mode's parser takes the tree at its first parse, so
;; the parse costs only what the mode did not cover.  The language is a
;; guess from `auto-mode-alist', which a file's local variables or magic
;; can overrule: a wrong one only costs the thread its work.
;;
;; Until the tree lands, redisplay shows the text plain and keys run: its
;; landing brings the faces (`treesit_parse_pending' in display.rs).  What
;; runs by itself must not ask for the tree meanwhile, or it waits for the
;; parse with the keys behind it: `show-paren-mode's idle timer is the one
;; such asker, and highlights no paren until the tree is there.

;;; Code:

(defconst romacs-treesit--mode-languages
  '((bash-ts-mode . bash) (c-ts-mode . c) (c++-ts-mode . cpp)
    (c-or-c++-ts-mode . c) (css-ts-mode . css) (go-ts-mode . go)
    (js-ts-mode . javascript) (json-ts-mode . json)
    (mhtml-ts-mode . html) (python-ts-mode . python)
    (rust-ts-mode . rust) (toml-ts-mode . toml) (tsx-ts-mode . tsx)
    (typescript-ts-mode . typescript) (yaml-ts-mode . yaml))
  "The language each tree-sitter mode's primary parser reads.")

(defconst romacs-treesit--early-minimum 4096
  "The fewest bytes worth a thread: a smaller file parses in less time.")

(defun romacs-treesit--file-language ()
  "The language the visited file's mode will parse, by its name, or nil."
  (let ((mode (assoc-default buffer-file-name auto-mode-alist #'string-match)))
    (when (consp mode) (setq mode (car mode)))
    (when mode (setq mode (major-mode-remap mode)))
    (when (and mode (symbolp mode))
      (alist-get (intern (string-remove-suffix "-maybe" (symbol-name mode)))
                 romacs-treesit--mode-languages))))

(defun romacs-treesit--visit (visit &rest args)
  "Around `after-find-file': call VISIT with ARGS, the parse running.
A parse no parser came for is abandoned, its thread left to end."
  (let* ((buffer (current-buffer))
         (language (and buffer-file-name enable-multibyte-characters
                        (>= (buffer-size) romacs-treesit--early-minimum)
                        (not (nth 4 args)) ; NOMODES
                        (romacs-treesit--file-language)))
         (started (and language (romacs--treesit-parse-early language))))
    (unwind-protect
        (apply visit args)
      (when (and started (buffer-live-p buffer))
        (with-current-buffer buffer
          (when (and enable-multibyte-characters
                     (not (treesit-parser-list nil nil t)))
            (romacs--treesit-parse-early nil)))))))

(defun romacs-treesit--no-parse-p (&rest _)
  "Whether the current buffer's tree is there to ask for."
  (not (romacs--treesit-parsing-p)))

(when (fboundp 'advice-add)
  (advice-add 'after-find-file :around #'romacs-treesit--visit)
  (with-eval-after-load 'treesit
    (advice-add 'treesit-show-paren-data :before-while #'romacs-treesit--no-parse-p)))

(provide 'romacs-treesit)
;;; romacs-treesit.el ends here
