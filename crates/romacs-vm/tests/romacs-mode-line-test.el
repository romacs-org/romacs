;;; romacs-mode-line-test.el --- Romacs's mode line: segments and ownership -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Run by tests/mode_line.rs in a session with GNU's Lisp, which `--batch' lacks.
(require 'romacs-mode-line)

(defun romacs-mode-line-test--format (format)
  "FORMAT expanded as a string over the current buffer, as a window's
mode line would be."
  (let ((noninteractive nil)) (substring-no-properties (format-mode-line format nil nil (current-buffer)))))

(defmacro romacs-mode-line-test--with-mode (&rest body)
  "Run BODY with `romacs-mode-line-mode' on, then turn it off."
  (declare (indent 0))
  `(unwind-protect (progn (romacs-mode-line-mode 1) ,@body)
     (romacs-mode-line-mode -1)))

(romacs-deftest romacs-mode-line-installs-over-gnu-and-restores-it ()
  (let ((gnu (default-value 'mode-line-format)))
    (romacs-should (eq gnu romacs-mode-line--gnu-default))
    (romacs-mode-line-test--with-mode
      (romacs-should (eq (default-value 'mode-line-format) romacs-mode-line-format)))
    (romacs-should (eq (default-value 'mode-line-format) gnu))))

(romacs-deftest romacs-mode-line-keeps-a-format-it-did-not-install ()
  (let ((gnu (default-value 'mode-line-format))
        (package (list "package" "%b")))
    (unwind-protect
        (romacs-mode-line-test--with-mode
          ;; A package installs its own after startup: off and on keep it.
          (setq-default mode-line-format package)
          (romacs-mode-line-mode -1)
          (romacs-should (eq (default-value 'mode-line-format) package))
          (romacs-mode-line-mode 1)
          (romacs-should (eq (default-value 'mode-line-format) package)))
      (setq-default mode-line-format gnu))))

(romacs-deftest romacs-mode-line-follows-classic-chrome ()
  (let ((gnu (default-value 'mode-line-format))
        (package (list "package")))
    (unwind-protect
        (progn
          (romacs-mode-line-mode 1)
          (romacs-classic-chrome 1)
          (romacs-should-not romacs-mode-line-mode)
          (romacs-should (eq (default-value 'mode-line-format) gnu))
          (romacs-classic-chrome -1)
          (romacs-should romacs-mode-line-mode)
          (romacs-should (eq (default-value 'mode-line-format) romacs-mode-line-format))
          ;; A package's format installed after startup survives classic.
          (setq-default mode-line-format package)
          (romacs-classic-chrome 1)
          (romacs-should (eq (default-value 'mode-line-format) package))
          (romacs-classic-chrome -1)
          (romacs-should (eq (default-value 'mode-line-format) package)))
      (romacs-classic-chrome -1)
      (romacs-mode-line-mode -1)
      (setq-default mode-line-format gnu))))

(romacs-deftest romacs-mode-line-leaves-a-buffers-own-format ()
  (with-temp-buffer
    (setq-local mode-line-format '("own"))
    (romacs-mode-line-test--with-mode
      (romacs-should (equal mode-line-format '("own"))))
    (romacs-should (equal mode-line-format '("own")))))

(romacs-deftest romacs-mode-line-gives-existing-buffers-their-values ()
  (let ((buffer (get-buffer-create "romacs-mode-line-existing")))
    (unwind-protect
        (romacs-mode-line-test--with-mode
          (with-current-buffer buffer
            (romacs-should (equal (substring-no-properties romacs-mode-line-buffer-name)
                                  "romacs-mode-line-existing"))))
      (kill-buffer buffer))))

(romacs-deftest romacs-mode-line-names-a-buffer-on-its-hooks ()
  (romacs-mode-line-test--with-mode
    (with-temp-buffer
      (rename-buffer "first-name" t)
      (run-hooks 'after-save-hook)
      (romacs-should (equal (substring-no-properties romacs-mode-line-buffer-name) "first-name"))
      ;; GNU's identification keymap rides along.
      (romacs-should (eq (get-text-property 0 'local-map romacs-mode-line-buffer-name)
                         mode-line-buffer-identification-keymap)))))

(romacs-deftest romacs-mode-line-counts-search-matches-when-isearch-publishes ()
  (romacs-mode-line-test--with-mode
    (with-temp-buffer
      (let ((isearch-mode t) (isearch-lazy-count-current nil) (isearch-lazy-count-total nil))
        (run-hooks 'lazy-count-update-hook)
        (romacs-should-not romacs-mode-line-search)
        (setq isearch-lazy-count-current 0 isearch-lazy-count-total 0)
        (run-hooks 'lazy-count-update-hook)
        (romacs-should-not romacs-mode-line-search)
        (setq isearch-lazy-count-current 3 isearch-lazy-count-total 12)
        (run-hooks 'lazy-count-update-hook)
        (romacs-should (equal (substring-no-properties romacs-mode-line-search) "  3/12")))
      (let ((isearch-mode nil))
        (run-hooks 'isearch-mode-end-hook)
        (romacs-should-not romacs-mode-line-search)))))

(romacs-mode-line-define-segment test-vc-probe
  "A probe refreshed by `vc-post-command-functions'."
  :compute (lambda () (and buffer-file-name (format "probe %d" (buffer-size))))
  :hooks (vc-post-command-functions))

(romacs-deftest romacs-mode-line-refreshes-the-buffers-a-vc-command-touched ()
  (let ((file (make-temp-file "romacs-mode-line-vc")))
    (unwind-protect
        (romacs-mode-line-test--with-mode
          (with-current-buffer (find-file-noselect file)
            (insert "abc")
            (run-hook-with-args 'vc-post-command-functions 'vc-next-action (list file) nil)
            (romacs-should (equal (substring-no-properties romacs-mode-line-test-vc-probe) "probe 3"))
            (set-buffer-modified-p nil)
            (kill-buffer)))
      (delete-file file))))

(romacs-deftest romacs-mode-line-shows-every-component-through-its-symbols ()
  (let ((file (make-temp-file "romacs-mode-line-all" nil ".el")))
    (unwind-protect
        (romacs-mode-line-test--with-mode
          (with-current-buffer (find-file-noselect file)
            ;; Shown in the selected window: `%l' and `%c' read its point.
            (switch-to-buffer (current-buffer))
            (dotimes (n 15) (insert (format "line %d\n" n)))
            (goto-char (point-min)) (forward-line 11) (forward-char 3)
            (narrow-to-region (point-min) (point-max))
            (narrow-to-region (1+ (point-min)) (point-max))
            (setq vc-mode " Git-main")
            (let* ((romacs-mode-line-minor-modes t)
                   (minor-mode-alist '((t " Probe")))
                   (text (romacs-mode-line-test--format romacs-mode-line-format)))
              (dolist (part (list (file-name-nondirectory file) "*" " Narrow" "12:3" " Git-main"
                                  (format-mode-line mode-name) " Probe"))
                (romacs-should (string-search part text))))
            (set-buffer-modified-p nil)
            (kill-buffer)))
      (delete-file file))))

(defvar romacs-mode-line-test--runs 0
  "How many times the process element below ran.")

(romacs-deftest romacs-mode-line-runs-a-process-element-once ()
  (romacs-mode-line-test--with-mode
    (with-temp-buffer
      (setq romacs-mode-line-test--runs 0)
      (let ((romacs-mode-line-minor-modes t)
            (mode-line-process '(:eval (progn (setq romacs-mode-line-test--runs (1+ romacs-mode-line-test--runs)) ":run"))))
        (romacs-should (string-search ":run" (romacs-mode-line-test--format romacs-mode-line-format)))
        (romacs-should (= romacs-mode-line-test--runs 1))))))

(romacs-deftest romacs-mode-line-has-one-space-at-the-right-align-marker ()
  (let ((romacs-mode-line-left '("L")) (romacs-mode-line-right '("R")))
    (romacs-mode-line-rebuild)
    (unwind-protect
        (romacs-should (equal (romacs-mode-line-test--format romacs-mode-line-format) "L R"))
      (custom-reevaluate-setting 'romacs-mode-line-left)
      (custom-reevaluate-setting 'romacs-mode-line-right)
      (romacs-mode-line-rebuild))))

(romacs-deftest romacs-mode-line-keeps-owning-its-rebuilt-format ()
  (let ((gnu (default-value 'mode-line-format)))
    (with-temp-buffer
      (unwind-protect
          (romacs-mode-line-test--with-mode
            (setq-local mode-line-format romacs-mode-line-format)
            (dolist (left '(("one") ("two")))
              (setopt romacs-mode-line-left left)
              (romacs-should (eq (default-value 'mode-line-format) romacs-mode-line-format))
              (romacs-should (eq mode-line-format romacs-mode-line-format))
              (romacs-should (string-prefix-p (concat (car left) " ") (romacs-mode-line-test--format mode-line-format))))
            (romacs-mode-line-mode -1)
            (romacs-should (eq (default-value 'mode-line-format) gnu))
            (romacs-should (eq mode-line-format gnu)))
        (custom-reevaluate-setting 'romacs-mode-line-left)
        (setq-default mode-line-format gnu)))))

(defvar romacs-mode-line-test--computed 0
  "How many times a test segment computed.")
(defvar romacs-mode-line-test-hook nil)
(defvar romacs-mode-line-test-other-hook nil)

(romacs-deftest romacs-mode-line-takes-segments-defined-while-on ()
  (romacs-mode-line-test--with-mode
    (with-temp-buffer
      (setq romacs-mode-line-test--computed 0)
      (eval '(romacs-mode-line-define-segment test-late "A late segment."
               :compute (lambda () (format "late %d" (setq romacs-mode-line-test--computed (1+ romacs-mode-line-test--computed))))
               :hooks (romacs-mode-line-test-hook))
            t)
      ;; It has its value at once, and its hook refreshes it.
      (romacs-should (string-prefix-p "late " romacs-mode-line-test-late))
      (let ((before romacs-mode-line-test--computed))
        (run-hooks 'romacs-mode-line-test-hook)
        (romacs-should (= romacs-mode-line-test--computed (1+ before))))
      ;; Redefined on another hook, the old one no longer refreshes it.
      (eval '(romacs-mode-line-define-segment test-late "A late segment."
               :compute (lambda () (format "late %d" (setq romacs-mode-line-test--computed (1+ romacs-mode-line-test--computed))))
               :hooks (romacs-mode-line-test-other-hook))
            t)
      (let ((before romacs-mode-line-test--computed))
        (run-hooks 'romacs-mode-line-test-hook)
        (romacs-should (= romacs-mode-line-test--computed before))
        (run-hooks 'romacs-mode-line-test-other-hook)
        (romacs-should (= romacs-mode-line-test--computed (1+ before)))))))

(romacs-mode-line-define-segment test-global
  "A global segment."
  :compute (lambda () (format "global %d" (setq romacs-mode-line-test--computed (1+ romacs-mode-line-test--computed))))
  :local nil)

(romacs-deftest romacs-mode-line-computes-a-global-segment-once ()
  (let ((buffers (list (generate-new-buffer "a") (generate-new-buffer "b") (generate-new-buffer "c"))))
    (unwind-protect
        (progn
          (setq romacs-mode-line-test--computed 0)
          (romacs-mode-line-test--with-mode
            (romacs-should (= romacs-mode-line-test--computed
                              ;; Once for the global one; once per buffer
                              ;; for the local test-late, if defined.
                              (1+ (if (fboundp 'romacs-mode-line-refresh-test-late) (length (buffer-list)) 0))))))
      (mapc #'kill-buffer buffers))))

(romacs-deftest romacs-icons-fall-back-without-their-font ()
  (let ((romacs-icons--font nil))
    (romacs-should (equal (romacs-icon 'modified) "●"))
    (romacs-should-not (romacs-icon 'lisp))
    (let ((romacs-icons-ascii t))
      (romacs-should (equal (romacs-icon 'modified) "*"))))
  (let ((romacs-icons--font t))
    (romacs-should (equal (romacs-icon 'modified) (string #xFE001)))
    (romacs-should (equal (get-text-property 0 'face (romacs-icon 'error 'error)) '(error romacs-icon)))
    ;; The mode line shows the character itself.
    (with-temp-buffer
      (emacs-lisp-mode)
      (romacs-mode-line-refresh-mode-icon)
      (romacs-should (string-prefix-p (string #xFE015 ?\s)
                                      (romacs-mode-line-test--format '("" romacs-mode-line-mode-icon)))))))

(provide 'romacs-mode-line-test)
;;; romacs-mode-line-test.el ends here
